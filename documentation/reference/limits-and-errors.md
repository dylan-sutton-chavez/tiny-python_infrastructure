---
title: "Limits and errors"
description: "Sandbox limits, error types, and runtime guarantees."
---

## Sandbox limits

Edge Python supports two limit profiles. Pick one when constructing the VM (`VM::with_limits` in Rust, `--sandbox` flag from the CLI).

| Limit          | `none()` (default) | `sandbox()`   | What hitting it raises |
|----------------|--------------------|---------------|------------------------|
| Max call depth | 1,000              | 256           | `RecursionError`       |
| Max operations | unbounded          | 100,000,000   | `RuntimeError`         |
| Max heap bytes | 10,000,000         | 100,000       | `MemoryError`          |

### Triggering limits

```python
# Recursion depth
def loop(n):
    return loop(n + 1)

try:
    loop(0)
except RecursionError:
    print("hit max depth")
```

```text Output
hit max depth
```

```python
# Heap quota — a tight loop allocating new objects
try:
    xs = []
    while True:
        xs = xs + [0] * 1000
except MemoryError:
    print("hit heap limit")
```

## Source size

The source file must be under **10 MiB**. Larger inputs are rejected at lex time.

## Token limits

| Limit                | Value |
|----------------------|-------|
| Max indent depth     | 100   |
| Max f-string depth   | 200   |
| Max expression depth | 200   |
| Max instructions per chunk | 65,535 |

These prevent pathological asymmetric DoS — a small input that produces an exponentially large parse tree or instruction stream.

## Error types

### Compile-time

Reported as `Diagnostic { line, col, end, msg }`. Caught before any code runs.

| Diagnostic                                | Cause                                  |
|-------------------------------------------|----------------------------------------|
| `expected X, got 'Y'`                     | Unexpected token                       |
| `'break' outside loop`                    | Misplaced control keyword              |
| `'continue' outside loop`                 | Misplaced control keyword              |
| `default 'except:' must be last`          | Bare `except` not at end               |
| `expression too deeply nested`            | Past `MAX_EXPR_DEPTH`                  |
| `program too large: exceeded maximum instruction limit` | Past `MAX_INSTRUCTIONS` |

### Runtime

Raised as `VmErr`. Most are catchable with `try` / `except`.

| Variant         | Class name           | When                               |
|-----------------|----------------------|------------------------------------|
| `Type`          | `TypeError`          | Wrong operand type                 |
| `Value`         | `ValueError`         | Right type, invalid value          |
| `Name`          | `NameError`          | Undefined name                     |
| `ZeroDiv`       | `ZeroDivisionError`  | Division or modulo by zero         |
| `CallDepth`     | `RecursionError`     | Past `max_calls`                   |
| `Heap`          | `MemoryError`        | Past heap limit                    |
| `Budget`        | `RuntimeError`       | Past op limit                      |
| `Runtime`       | `RuntimeError`       | Internal invariant or unsupported  |
| `Raised`        | `Exception`          | User `raise X` with non-builtin X  |

### Catching errors

```python
def safe(f, x):
    try:
        return f(x)
    except TypeError:
        return "type"
    except ValueError:
        return "value"
    except ZeroDivisionError:
        return "zero"
    except:
        return "other"

print(safe(lambda x: 1 / x, 0))
print(safe(lambda x: int(x), "abc"))
print(safe(lambda x: len(x), 42))
```

```text Output
zero
value
type
```

## Unsupported features at runtime

These parse but raise `RuntimeError` when executed.

```python
# Classes
try:
    class Foo:
        pass
except RuntimeError as e:
    print("class:", e)
```

```python
# Imports
try:
    import os
except RuntimeError as e:
    print("import:", e)
```

These exist for syntactic compatibility — your code can be lifted from CPython without parsing failing — but the VM rejects them when reached. If you need a value-oriented record, use a dict or a tuple. If you need code reuse, use higher-order functions.

## Determinism

For a given source program and input, Edge Python produces the same output across runs and across architectures (`x86_64`, `aarch64`, `wasm32`). There is no time, no randomness, no thread scheduling, no OS interaction. The only source of nondeterminism is the heap pool's slot reuse, which is observable through `id(x)` only — never through `==`, `repr`, or any other operation.