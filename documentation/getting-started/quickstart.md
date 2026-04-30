---
title: "Quickstart"
description: "Run your first Edge Python program in under a minute."
---

## Install

Three ways to get Edge Python running.

### Browser playground

The fastest path. No install, runs entirely client-side via WebAssembly.

[Open the playground →](https://demo.edgepython.com)

### Native binary

Pre-built for Linux, macOS, and Windows on the [releases page](https://github.com/dylan-sutton-chavez/edge-python/releases). Download, make executable, run.

```bash
chmod +x edge-linux-x86_64
./edge-linux-x86_64 -c 'print("hello")'
```

```text Output
hello
```

### Build from source

Requires a recent stable Rust toolchain.

```bash
git clone https://github.com/dylan-sutton-chavez/edge-python
cd edge-python/compiler
cargo build --release
./target/release/edge -c 'print("hello")'
```

## CLI usage

```bash
edge [options] <file>
edge -c <code>
```

| Option       | Effect                                             |
|--------------|----------------------------------------------------|
| `-c <code>`  | Run inline code instead of reading a file          |
| `--sandbox`  | Enforce hard limits on heap, ops, and call depth   |
| `-d` / `-dd` | Debug output — IC stats and heap footprint         |
| `-q`         | Suppress info logs                                 |
| `-h`         | Show help                                          |

## Your first program

Save this as `hello.py`:

```python
greet = lambda name: f"Hello, {name}!"

for who in ["world", "edge", "python"]:
    print(greet(who))
```

Run it:

```bash
edge hello.py
```

```text Output
Hello, world!
Hello, edge!
Hello, python!
```

## A taste of the language

Edge Python is a functional subset of Python 3.13. Functions are first-class values. Lambdas, currying, higher-order functions, and comprehensions are central.

```python
# First-class functions
ops = [abs, len, str]
print([f(-3) for f in ops])

# Currying
add = lambda x: lambda y: x + y
print(add(3)(4))

# Pure functions get memoized automatically
def fib(n):
    if n < 2: return n
    return fib(n - 1) + fib(n - 2)

print(fib(20))
```

```text Output
[3, 2, '-3']
7
6765
```

## What's next

<CardGroup cols={2}>
  <Card title="What it is" icon="compass" href="/getting-started/what-it-is">
    Scope, paradigm, and what intentionally isn't supported.
  </Card>
  <Card title="Syntax" icon="code" href="/language/syntax">
    Operators, literals, and the language surface.
  </Card>
  <Card title="Built-ins" icon="package" href="/reference/builtins">
    Every built-in function with examples and outputs.
  </Card>
  <Card title="Methods" icon="list" href="/reference/methods">
    String, list, and dict methods.
  </Card>
</CardGroup>