## Edge Python

Single-pass SSA compiler for Python 3.13: hand-written lexer, token-to-bytecode parser, adaptive VM with inline caching, template memoization, and configurable sandbox limits.

---

### Architecture

- **Lexer**: Hand-written scanner, LUT-based, Python 3.13 tokens
- **Parser**: Single-pass SSA, phi nodes, precedence climbing, direct bytecode emission
- **VM**: Adaptive stack machine, NaN-boxed values, inline caching, template memoization
- **Sandbox**: Configurable recursion, operation, and heap limits

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

Recursive Fibonacci — `fib(45)`:

```python
def fib(n):
    if n < 2: return n
    return fib(n-1) + fib(n-2)
print(fib(45))
```

| Runtime      | fib(45) real | fib(45) user | sys      | fib(90) real |
|--------------|--------------|--------------|----------|--------------|
| CPython 3.13 | 1m56.345s    | 1m56.324s    | 0m0.009s | n/a          |
| Edge Python  | 0m0.011s     | 0m0.000s     | 0m0.003s | 0m0.013s     |

*10,577x faster than CPython on recursive fib(45), where fib(90) completes in 13ms.*

One Million Iterations — `1_000_000`:

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

| Command                         | Description                                       |
|---------------------------------|---------------------------------------------------|
| `edge script.py`                | Run with no limits                                |
| `edge --sandbox script.py`      | Run with sandbox (512 calls, 100M ops, 100K heap) |
| `edge -d --sandbox script.py`   | Debug output (verbosity level 1)                  |
| `edge -dd --sandbox script.py`  | Debug output (verbosity level 2)                  |

### Building for WebAssembly

```bash
rustup target add wasm32-unknown-unknown
cargo build --target wasm32-unknown-unknown --release --no-default-features --features wasm
```

*Exported functions: `src_ptr()`, `out_ptr()`, `run(len: usize)` -> `usize`*

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

### License

MIT OR Apache-2.0