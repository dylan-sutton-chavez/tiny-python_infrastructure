## Edge Python

Single-pass SSA compiler for Python 3.13: hand-written lexer, token-to-bytecode parser, adaptive virtual machine with NaN-boxed values, inline caching, template memoization, mark-sweep garbage collector, and configurable sandbox limits. Native and WASM targets.

---

### Architecture

- **Lexer**: Hand-written scanner, LUT-based, Python 3.13 tokens
- **Parser**: Single-pass SSA (static single assignment with $\phi$-nodes), Pratt precedence climbing, direct bytecode emission
- **VM**: Adaptive stack machine, NaN-boxed values, inline caching, template memoization
- **Sandbox**: Configurable recursion, operation, and heap limits
- **Garbage Collector**: Mark-and-sweep with string interning ($\leq 64$ bytes), free-list reuse, threshold-based triggering

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

Recursive Fibonacci — $\text{fib}(45)$ (pure-function memoization after 4 calls reduces $O(2^n)$ to $O(n)$ complexity):

```python
def fib(n):
    if n < 2: return n
    return fib(n-1) + fib(n-2)
print(fib(45))
```

| Runtime      | $\text{fib}(45)$ real | $\text{fib}(45)$ user | sys      | $\text{fib}(90)$ real |
|--------------|------------------------|------------------------|----------|------------------------|
| CPython 3.13 | 1m56.345s              | 1m56.324s              | 0m0.009s | n/a                    |
| Edge Python  | 0m0.011s               | 0m0.000s               | 0m0.003s | 0m0.013s               |

One Million Iterations — $10^6$:

```python
counter: int = 0
for _ in range(1_000_000):
    counter += 1
print(counter)
```

| Runtime      | real     | user     | sys      |
|--------------|----------|----------|----------|
| CPython 3.13 | 0m0.058s | 0m0.041s | 0m0.008s |
| Edge Python  | 0m0.056s | 0m0.054s | 0m0.001s |

### Usage

| Command                         | Description                                             |
|---------------------------------|---------------------------------------------------------|
| `edge script.py`                | Run with no limits                                      |
| `edge --sandbox script.py`      | Run with sandbox ($512$ calls, $10^8$ ops, $10^5$ heap) |
| `edge -d --sandbox script.py`   | Debug output (verbosity level 1)                        |
| `edge -dd --sandbox script.py`  | Debug output (verbosity level 2)                        |
| `edge -q script.py`             | Quiet mode (suppresses compiler diagnostics)            |

### Value Representation

NaN-boxed 64-bit: integers are 48-bit signed ($\pm 2^{47}$), overflow promotes to float. Heap index is 28-bit ($2^{28}$ objects max, returns `MemoryError` beyond).

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
│   │       ├── mod.rs
│   │       ├── types.rs
│   │       ├── cache.rs
│   │       ├── ops.rs
│   │       ├── builtins.rs
│   │       └── collections.rs
│   └── wasm.rs
└── tests
    ├── cases
    │   ├── lexer_cases.json
    │   ├── parser_cases.json
    │   └── vm_cases.json
    ├── integration_test.rs
    ├── lexer_test.rs
    ├── parser_test.rs
    └── vm_test.rs
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
6. Hölzle & Ungar, *Optimizing Dynamically-Dispatched Calls with Run-Time Type Feedback* (PLDI 1994). Adaptive rewriting.
7. Michie, *Memo Functions and Machine Learning* (Nature 1968). Memoization.
8. McCarthy, *Recursive Functions of Symbolic Expressions* (CACM 1960). Mark-sweep garbage collector.
9. Shannon, PEP 659: *Specializing Adaptive Interpreter* (2021). Tiered specialization.

### License

MIT OR Apache-2.0