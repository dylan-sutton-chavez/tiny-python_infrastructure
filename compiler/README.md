# Edge Python

A high-performance, single-pass SSA compiler and virtual machine based on the CPython 3.13 specification. It features a hand-written lexer, a Pratt-precedence parser with direct SSA-to-bytecode emission, and a three-tier adaptive VM designed for deterministic execution and extreme safety in sandboxed environments.

* **Demo:** [demo.edgepython.com](https://demo.edgepython.com/)
* **Docs:** [edgepython.com](https://edgepython.com/)

---

## 1. Architecture Overview

The apporach for using Static Single Assignment (SSA) even at the bytecode level, allowing a compile-time and runtime optimizations that typically require a full JIT maintaining a syntax parsing with linear time complexity.

* **Lexer**: Hand-written, LUT-based scanner implementing the CPython 3.13 token specification.
* **Parser**: Single-pass SSA engine using Pratt precedence climbing. It bypasses intermediate ASTs to emit bytecode directly with $\phi$-node resolution.
* **VM (Threaded Adaptive)**:
    * **Threaded Code**: Pre-compiled `ThreadedOp` array with baked-in operands eliminates the dispatch `match` overhead. Each instruction is a direct enum variant — no opcode lookup, no indirect branch through a jump table.
    * **Inline Caching (IC)**: Type recording with adaptive specialization, promoting hot arithmetic and comparisons to type-specialized fast paths after $4$ stable hits.
* **Memory**: NaN-boxed 64-bit values with an arbitrary-precision `BigInt` fallback and a mark-and-sweep garbage collector.

---

## 2. Compiler Design

Maintaining a **SSA store convention**. Every local variable mutation is treated as a new versioning of a slot. Where:

1. **Phi-Resolution**: Control flow merges use explicit $\phi$-nodes to resolve variable versions (eg., `x` -> `x_1`, `x_2`).
2. **Soundness**: By enforcing SSA invariants in the bytecode, we avoid the "hidden state" bugs common in register-based VMs.
3. **Optimization Trigger**: The compiler performs a single constant-folding pass at parse time. If a loop matches a known induction pattern (like `for i in range(N)`), the SSA graph enables constant folding and closed-form loop evaluation at compile time.

---

## 3. Optimization

**Why Threaded Code Instead of Trace/Method JITs**

While CPython 3.13 explores copy-and-patch Tier-2 JITs, Edge Python uses **threaded code** with inline caching for the following reasons:

**Threaded Code Eliminates Dispatch Overhead** The bytecode is pre-compiled into a `ThreadedOp` array where each instruction carries its operand. The VM executes a flat match over pre-resolved variants — no opcode lookup, no indirect branch through a jump table. This removes the primary bottleneck of traditional interpreters (Ertl \& Gregg, 2003).

**Inline Caching Specializes Hot Paths** Arithmetic and comparison operations record operand types. After 4 stable hits, the IC promotes to type-specialized fast paths (e.g., `AddInt`, `LtFloat`) that skip type checks entirely.

**SSA Integrity** Trace executors often bypass the SSA store convention to gain speed. In this architecture, that would silently corrupt $\phi$ resolution after deoptimizations. Threaded code preserves SSA invariants by design.

**Maintenance and Portability**

Edge Python is a $\pm 130$ KB embedded interpreter.

* **Method JITs** require platform-specific assembly stencils.
* **Trace JITs** introduce a second execution model that must stay synchronized with the bytecode contract, garbage collector, and built-ins.

*Staying "using just Rust" ensures identical behavior across `x86_64`, `aarch64`, and `wasm32`.*

---

## 4. Benchmark

Testing Ten Million Iterations ($10^7$):

```python
counter: int = 0
for _ in range(10_000_000): counter += 1
print(counter)
```

| Runtime          | Real Time  | Logic                    |
|------------------|------------|--------------------------|
| **CPython 3.13** | 1.180s     | Standard Bytecode Loop   |
| **Edge Python**  | **0.010s** | Threaded code + IC |

---

## 5. Technical Implementation

### Value Representation

Utilize **NaN-boxing** (64-bit).

* **Integers**: 48-bit signed ($\pm 2^{47}$) stored inline.
* **BigInt**: Fallback for values $> 48$-bit; uses a base-$2^{32}$ limb array.
* **Floats**: Standard IEEE 754.
* **Heap**: 28-bit index ($2^{28}$ max objects).

### Garbage Collection

A **Mark-and-Sweep** collector handles heap management:

* **String Interning**: Applied to all strings $\leq 64$ bytes.
* **Thresholds**: Configurable memory pressure triggers.
* **Sandbox**: Hard limits on recursion depth, total operations, and heap size.

---

## 6. Project Structure

```text
├── Cargo.lock
├── Cargo.toml
├── README.md
├── src
│   ├── lib.rs
│   ├── main.rs
│   ├── modules
│   │   ├── fstr.rs
│   │   ├── fx.rs
│   │   ├── lexer
│   │   │   ├── mod.rs
│   │   │   ├── scan.rs
│   │   │   └── tables.rs
│   │   ├── parser
│   │   │   ├── control.rs
│   │   │   ├── expr.rs
│   │   │   ├── literals.rs
│   │   │   ├── mod.rs
│   │   │   ├── stmt.rs
│   │   │   └── types.rs
│   │   └── vm
│   │       ├── builtins.rs
│   │       ├── cache.rs
│   │       ├── collections.rs
│   │       ├── handlers
│   │       │   ├── arith.rs
│   │       │   ├── attr.rs
│   │       │   ├── data.rs
│   │       │   ├── function.rs
│   │       │   ├── mod.rs
│   │       │   └── unsupported.rs
│   │       ├── mod.rs
│   │       ├── ops.rs
│   │       ├── optimizer.rs
│   │       ├── threaded.rs
│   │       └── types.rs
│   └── wasm.rs
└── tests
    ├── cases
    │   ├── lexer.json
    │   ├── parser.json
    │   └── vm.json
    ├── lexer.rs
    ├── main.rs
    ├── parser.rs
    └── vm.rs
```

## 7. Quick Start

```bash
# Build the native binary
cargo build --release

# Run a simple script
./target/release/edge -c 'print("Edge Python Online")'

# Run in Sandbox Mode
./target/release/edge --sandbox script.py
```

---

## 8. References

1.  **Aho, Sethi & Ullman**, *Compilers: Principles, Techniques and Tools* (1986). LUT-based lexer.
2.  **Pratt**, *Top Down Operator Precedence* (POPL 1973). Precedence climbing parser.
3.  **Cytron et al.**, *Efficiently Computing Static Single Assignment Form* (TOPLAS 1991). SSA, $\phi$-nodes.
4.  **Gudeman**, *Representing Type Information in Dynamically Typed Languages* (1993). NaN-boxing.
5.  **Deutsch & Schiffman**, *Efficient Implementation of the Smalltalk-80 System* (POPL 1984). Inline caching.
6.  **Ertl & Gregg**, *The Structure and Performance of Efficient Interpreters* (JILP 2003). Bytecode dispatch.
7.  **Hölzle & Ungar**, *Optimizing Dynamically-Dispatched Calls with Run-Time Type Feedback* (PLDI 1994).
8.  **Brunthaler**, *Inline Caching Meets Quickening* (ECOOP 2010). Per-bytecode specialization.
9.  **Casey et al.**, *Towards Superinstructions for Java Interpreters* (SCOPES 2003). Superinstruction fusion.
10. **Michie**, *Memo Functions and Machine Learning* (Nature 1968). Template memoization.
11. **McCarthy**, *Recursive Functions of Symbolic Expressions* (CACM 1960). Mark-sweep GC.
12. **Knuth**, *The Art of Computer Programming, Vol. 2* (1981). Arbitrary-precision arithmetic.
13. **Shannon**, *PEP 659: Specializing Adaptive Interpreter* (2021). Tiered specialization (CPython 3.11+).
14. **O'Connor**, *PEP 709: Inlined Comprehensions* (2023). Drain-and-reinject compilation.
15. **Xu & Kjolstad**, *Copy-and-Patch Compilation* (OOPSLA 2021). Basis for CPython 3.13's Tier-2 JIT.