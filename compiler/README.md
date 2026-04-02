## Edge Python

Single-pass SSA compiler for Python 3.13: logos lexer, token-to-bytecode parser, adaptive VM with inline caching, template memoization, and configurable sandbox limits.

---

### Architecture

- **Lexer**: DFA-driven tokenization, offset-indexed, zero-alloc
- **Parser**: Single-pass SSA, phi nodes, precedence climbing, direct bytecode emission
- **VM**: Adaptive stack machine, inline caching, template memoization
- **Sandbox**: Configurable recursion, operation, and heap limits

### Quick Start

```bash
cd compiler/

cargo build --release
./target/release/edge -c 'print("Hello, world!")'
```

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
├── main.py
├── README.md
├── src
│   ├── lib.rs
│   ├── main.rs
│   ├── modules
│   │   ├── lexer.rs
│   │   ├── parser.rs
│   │   └── vm.rs
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