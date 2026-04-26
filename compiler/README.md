## Edge Python

Single-pass SSA compiler based on CPython 3.13: hand-written lexer, token-to-bytecode parser, three-tier adaptive virtual machine with NaN-boxed values, inline caching, superinstruction fusion, template memoization, mark-sweep garbage collector, and configurable sandbox limits. Native and WASM targets.

* **Demo:** *[demo.edgepython.com](https://demo.edgepython.com/)*
* **Docs:** *[edgepython.com](https://demo.edgepython.com/)*

---

### Architecture

* **Lexer**: Hand-written scanner, LUT-based, CPython 3.13 tokens
* **Parser**: Single-pass SSA (static single assignment with $\phi$-nodes), Pratt precedence climbing, direct bytecode emission
* **VM**: Three-tier adaptive interpreter
  * **Tier-0**: flat opcode dispatch (LLVM jump table, single indirect branch per instruction)
  * **Tier-1**: inline caching with type recording, promoted to specialized ops after $8$ stable hits
  * **Tier-2**: superinstruction fusion at chunk creation; pattern catalog includes `Inc`, `Lt`, `LoopGuard`, and `RangeIncFused` (closed-form $O(1)$ evaluation of `for _ in range(N): x += k`)
* **Sandbox**: Configurable recursion, operation, and heap limits
* **Garbage Collector**: Mark-and-sweep with string interning ($\leq 64$ bytes), free-list reuse, threshold-based triggering

### Quick Start

Build and Install:

```bash
cd compiler/

cargo build --release
./target/release/edge -c 'print("Hello, world!")'
```

Add to `$PATH`:

```bash
realpath target/release/edge

echo 'export PATH="/path/to/compiler/target/release:$PATH"' >> ~/.bashrc
source ~/.bashrc
```

### Benchmarks

Ten Million Iterations ($10^7$):

```python
counter: int = 0
for _ in range(10_000_000): counter += 1
print(counter)
```

| Runtime          | real     | user     | sys      |
|------------------|----------|----------|----------|
| CPython 3.13     | 0m1.180s | 0m1.150s | 0m0.020s |
| Edge Python      | 0m0.010s | 0m0.000s | 0m0.003s |

The fused `RangeIncFused` superinstruction collapses the loop to a single i128 multiplication. Programs that don't match a fused pattern fall back to tier-1 IC (typically $30$-$50$% faster than the baseline).

### Usage

| Command                         | Description                                             |
|---------------------------------|---------------------------------------------------------|
| `edge script.py`                | Run with no limits                                      |
| `edge --sandbox script.py`      | Run with sandbox ($512$ calls, $10^8$ ops, $10^5$ heap) |
| `edge -d --sandbox script.py`   | Debug output (verbosity level 1)                        |
| `edge -dd --sandbox script.py`  | Debug output (verbosity level 2)                        |
| `edge -q script.py`             | Quiet mode (suppresses compiler diagnostics)            |

### Value Representation

NaN-boxed 64-bit: integers are 48-bit signed ($\pm 2^{47}$) for inline storage; values outside this range are heap-allocated as arbitrary-precision `BigInt` (base-$2^{32}$ limb array, sign-magnitude), matching Python's unbounded `int` semantics. True division (`/`) always yields `float`. Heap index is 28-bit ($2^{28}$ objects max, returns `MemoryError` beyond).

### Building for WebAssembly

```bash
rustup target add wasm32-unknown-unknown
cargo build --target wasm32-unknown-unknown --release --no-default-features --features wasm
```

*Exported functions: `src_ptr()`, `out_ptr()`, `run(len: usize)` $\rightarrow$ `usize`*

### Project Structure

```bash
├── Cargo.lock
├── Cargo.toml
├── README.md
├── src
│   ├── lib.rs
│   ├── main.rs
│   ├── modules
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
│   │       ├── super_ops.rs
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

### Tests

```bash
cargo test
cargo test -- --ignored
cargo test --features wasm-tests
```

### References

1. Aho, Sethi & Ullman, *Compilers: Principles, Techniques and Tools* (1986). LUT-based lexer.
2. Pratt, *Top Down Operator Precedence* (POPL 1973). Precedence climbing parser.
3. Cytron, Ferrante, Rosen, Wegman & Zadeck, *Efficiently Computing Static Single Assignment Form* (TOPLAS 1991). SSA, $\phi$-nodes.
4. Gudeman, *Representing Type Information in Dynamically Typed Languages* (1993). NaN-boxing.
5. Deutsch & Schiffman, *Efficient Implementation of the Smalltalk-80 System* (POPL 1984). Inline caching.
6. Ertl & Gregg, *The Structure and Performance of Efficient Interpreters* (JILP 2003). Bytecode dispatch, indirect branch prediction.
7. Hölzle & Ungar, *Optimizing Dynamically-Dispatched Calls with Run-Time Type Feedback* (PLDI 1994). Adaptive rewriting.
8. Brunthaler, *Inline Caching Meets Quickening* (ECOOP 2010). Per-bytecode specialization with trivial deopt.
9. Casey, Gregg, Ertl & Nisbet, *Towards Superinstructions for Java Interpreters* (SCOPES 2003). Superinstruction fusion.
10. Michie, *Memo Functions and Machine Learning* (Nature 1968). Memoization.
11. McCarthy, *Recursive Functions of Symbolic Expressions* (CACM 1960). Mark-sweep garbage collector.
12. Knuth, *The Art of Computer Programming, Vol. 2: Seminumerical Algorithms* (1981). Arbitrary-precision arithmetic, §4.3.
13. Shannon, PEP 659: *Specializing Adaptive Interpreter* (2021). Tiered specialization, CPython 3.11+.
14. O'Connor, PEP 709: *Inlined Comprehensions* (2023). Drain-and-reinject compilation.
15. Xu & Kjolstad, *Copy-and-Patch Compilation* (OOPSLA 2021). Stencil-based fast JIT, basis for CPython 3.13's tier-2.

### License

MIT OR Apache-2.0