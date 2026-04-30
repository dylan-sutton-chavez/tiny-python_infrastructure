---
title: "Design"
description: "Compiler architecture, dispatch model, and runtime layout."
---

## Overview

Edge Python is a compact bytecode compiler and stack VM for a functional subset of Python 3.13. The release build is approximately 130 KB on `wasm32-unknown-unknown` with `panic=abort` and `opt-level=z`. The codebase is organized as a hand-written lexer, a single-pass Pratt parser that emits SSA-versioned bytecode directly, a peephole optimizer for constant folding, and a token-threaded interpreter with two layers of adaptive specialization on top.

There is no AST and no IR: bytecode is the only intermediate representation between source and execution.

## Concepts

- **Offset-based tokens**: Tokens carry `(start, end, kind)` indices into the source buffer. No string copies during lexing; identifier and string content is sliced lazily by the parser.
- **Single-pass SSA codegen**: Variables are versioned per assignment (`x` -> `x_1`, `x_2`). Control-flow joins emit explicit `Phi` opcodes resolved at runtime.
- **Token-threaded dispatch**: The instruction stream is `Vec<Instruction>` where each `Instruction` is `(opcode: OpCode, operand: u16)`. The hot loop is a flat `match` on the opcode variant. Rust lowers it to a jump table; this is *token threading*, not direct threading (computed-goto is not available in safe Rust).
- **Per-instruction inline caching**: Each binary op records the type tags of its operands. After 4 stable hits the IC stores a typed `FastOp` (e.g. `AddInt`, `LtFloat`) used as a speculative fast path with a type-guard deopt.
- **Template memoization**: Pure user functions cache results keyed by their argument tuple. After 2 hits the cached value short-circuits execution. Functions are statically classified as pure/impure during emission, and the runtime tightens the classification by observing `StoreItem`, `StoreAttr`, `Raise`, etc.
- **NaN-boxed values**: `Val` is a 64-bit union encoding ints, floats, bools, None, and 28-bit heap indices in a single word.
- **Mark-and-sweep GC**: Triggered when the heap crosses an adaptive threshold. Roots include the stack, globals, iterator frames, the current slot window, and saved live-slot snapshots.

## Bytecode shape

Each `Instruction` is 4 bytes: a 1-byte `OpCode` discriminant (with `#[repr(u8)]` planned), a 2-byte operand, and 1 byte of padding. Opcodes fall into 17 categories — load, store, arith, bitwise, compare, logic, identity, control flow, iter, build, container, comprehension, function, ssa (Phi), yield, side effects, and unsupported (raises at runtime).

```text
OpCode::LoadConst    operand = constant index
OpCode::LoadName     operand = name slot
OpCode::StoreName    operand = name slot
OpCode::Add / Sub    operand = 0 (IC slot derived from ip)
OpCode::Call         operand = (kw << 8) | pos
OpCode::Phi          operand = target slot, sources in chunk.phi_sources
OpCode::ForIter      operand = jump target on iterator exhaustion
```

## Dispatch shape

The hot loop reads `cache.fused_ref()[ip]` — a snapshot of the instruction stream where adjacent `LoadAttr + Call` pairs have been fused into the `CallMethod + CallMethodArgs` superinstruction. This fusion is performed once per chunk, cached, and reused across calls.

For arithmetic and comparison opcodes, the loop first checks `cache.get_fast(ip)`. If a `FastOp` is present, the speculative path runs inline and pops two operands without a function call. On a type-guard miss the cache is invalidated and execution falls back to the generic handler. The IC is per-instruction, so monomorphic call sites stabilize independently.

## Memory model

`Val` is 64 bits NaN-boxed:

| Tag       | Pattern                                 | Notes                        |
|-----------|-----------------------------------------|------------------------------|
| Float     | any non-canonical IEEE-754              | Quiet NaN remapped           |
| Int       | `QNAN \| SIGN \| i48`                   | ±2⁴⁷ inline; BigInt above    |
| None      | `QNAN \| 1`                             |                              |
| True      | `QNAN \| 2`                             |                              |
| False     | `QNAN \| 3`                             |                              |
| Heap      | `QNAN \| 4 \| (i28 << 4)`               | 28-bit index into `HeapPool` |

The heap is an arena of `Option<HeapObj>` slots with a free list. Strings of 64 bytes or fewer are interned in a side hash. Integers above 2⁴⁷ are promoted to `BigInt`, a base-2³² little-endian limb array with Knuth Algorithm D for division. The garbage collector is a single-color mark-and-sweep that runs when `live > gc_threshold` or `alloc_count > max(live/4, 4096)`.

## What the compiler intentionally does *not* do

- No SSA-wide constant propagation through `LoadName`. The load is preserved because removing it pessimizes the IC, super-op, and template paths.
- No CSE, GVN, LICM, inlining, or closed-form loop folding.
- No dead-store elimination beyond what falls out of constant folding.
- No IR — there is exactly one representation between source and dispatch.
- No JIT. Edge Python stays single-tier and pure Rust. Method JITs need per-architecture stencils; trace JITs duplicate the execution model and complicate the GC contract.
- No object model. `class` parses but `MakeClass` raises at runtime — the language is functional.
- No module system. `import` and `from ... import` parse but raise at runtime.

## Architecture

```text
src/
 ├── main.rs
 ├── modules
 │   ├── fstr.rs
 │   ├── fx.rs
 │   ├── lexer
 │   │   ├── mod.rs
 │   │   ├── scan.rs
 │   │   └── tables.rs
 │   ├── parser
 │   │   ├── control.rs
 │   │   ├── expr.rs
 │   │   ├── literals.rs
 │   │   ├── mod.rs
 │   │   ├── stmt.rs
 │   │   └── types.rs
 │   └── vm
 │       ├── builtins.rs
 │       ├── cache.rs
 │       ├── handlers
 │       │   ├── arith.rs
 │       │   ├── data.rs
 │       │   ├── function.rs
 │       │   ├── methods.rs
 │       │   └── mod.rs
 │       ├── mod.rs
 │       ├── ops.rs
 │       ├── optimizer.rs
 │       └── types.rs
 └── wasm.rs
```

## Capabilities

| Types  | Control flow     | Built-ins         | Lexical         |
|--------|------------------|-------------------|-----------------|
| int    | if / elif / else | I/O               | indentation     |
| float  | for / while      | type conversion   | f-string        |
| str    | match / case     | introspection     | walrus operator |
| bool   | functions        | iteration         | comments        |
| list   | lambdas          | aggregation       | docstrings      |
| dict   | generators       | math              | underscore      |
| tuple  | comprehensions   | sequence ops      | complex numbers |
| set    | try / except     | logical reduction | escape sequences|
| range  | with             | number formatting | -               |
| None   | async / await¹   | -                 | -               |
| BigInt | yield / yield from | -                | -               |

¹ async syntax parses and emits `MakeCoroutine` for compatibility, but there is no event loop — coroutines run synchronously.

## References

1. Aho, Sethi & Ullman. *Compilers: Principles, Techniques and Tools* (1986). LUT-based lexer.
2. Pratt. *Top Down Operator Precedence* (POPL 1973).
3. Cytron et al. *Efficiently Computing Static Single Assignment Form* (TOPLAS 1991).
4. Gudeman. *Representing Type Information in Dynamically Typed Languages* (1993). NaN-boxing.
5. Deutsch & Schiffman. *Efficient Implementation of the Smalltalk-80 System* (POPL 1984). Inline caching.
6. Ertl & Gregg. *The Structure and Performance of Efficient Interpreters* (JILP 2003). Threaded dispatch.
7. Casey et al. *Towards Superinstructions for Java Interpreters* (SCOPES 2003). LoadAttr+Call fusion.
8. Michie. *Memo Functions and Machine Learning* (Nature 1968). Pure-function memoization.
9. McCarthy. *Recursive Functions of Symbolic Expressions* (CACM 1960). Mark-sweep GC.
10. Knuth. *The Art of Computer Programming, Vol. 2* (1981). Algorithm D for BigInt division.
11. Backus. *Can Programming Be Liberated from the von Neumann Style?* (CACM 1978). Function-level paradigm.