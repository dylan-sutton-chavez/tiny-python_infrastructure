# Edge Python

A compact, single-pass SSA bytecode compiler and stack VM for a functional subset of CPython 3.13 syntax. Hand-written lexer, Pratt parser that emits bytecode directly, and a threaded-code interpreter with per-instruction inline caching and pure-function memoization.

Built for deterministic execution in sandboxed and embedded environments. The release WASM build is ~130 KB.

- **Demo:** [demo.edgepython.com](https://demo.edgepython.com/)
- **Docs:** [edgepython.com](https://edgepython.com/)

## Repository layout

```text
# Rust crate: lexer, parser, optimizer, VM
compiler/

# Browser playground (HTML + WASM + Web Worker)
demo/

# Mintlify documentation source
documentation/

# CI/CD pipelines (lint, native builds, WASM, demo)
.github/
```

## Quick start

```bash
# Native binary
cd compiler
cargo build --release
./target/release/edge -c 'print((lambda x: x * 2)(21))'

# Run a file with sandbox limits
./target/release/edge --sandbox script.py
```

Pre-built binaries for Linux, macOS, and Windows are available on the [releases page](https://github.com/dylan-sutton-chavez/edge-python/releases).

## What it is

Edge Python targets functional edge computing: first-class functions, lambdas, closures, generators, comprehensions, and pure-function memoization. Classes and imports parse for compatibility but raise at runtime. There is no object model and no module system.

For architecture details, see [`compiler/README.md`](compiler/README.md). For language reference and implementation notes, see the [docs](https://edgepython.com/).

## License

MIT OR Apache-2.0