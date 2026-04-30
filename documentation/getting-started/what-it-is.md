---
title: "What Edge Python is"
description: "A functional subset of Python 3.13, compiled to bytecode and run on a sandboxed VM."
---

Edge Python is a compiler and stack VM for a **functional subset of CPython 3.13 syntax**. It targets edge computing: deterministic execution, hard sandbox limits, no I/O surface beyond `print`, and a release size around 130 KB compiled to WebAssembly.

The language reads like Python because it parses Python's syntax. It runs differently because what it executes is curated.

## What it supports

- **First-class functions**: pass them, return them, store them in lists and dicts.
- **Lambdas with closures**: full lexical capture, currying, partial application.
- **Generators**: `yield`, `yield from`, generator expressions.
- **Comprehensions**: list, dict, set, with multiple `for` clauses and guards.
- **Pattern matching**: `match` / `case`, including `_` wildcard.
- **Exceptions**: `try` / `except` / `else` / `finally`, with named handlers and `raise from`.
- **Context managers**: `with` blocks, single and multi-target.
- **Numbers**: arbitrary-precision integers (BigInt fallback past 2⁴⁷), IEEE-754 floats.
- **Sequences**: lists, tuples, dicts (insertion-ordered), sets, ranges, strings.
- **f-strings**: with format specs, embedded expressions, and `!r` / `!s` flags.
- **Walrus operator**: `:=` in expressions.
- **Type annotations**: parsed and ignored, like CPython for non-strict tools.

## What it doesn't support

These parse for syntactic compatibility but raise at runtime, or simply don't exist:

- **Classes and objects**: `class` parses, `MakeClass` raises. No inheritance, no instance state, no method resolution. Edge Python is functional — model your domain with closures, dicts, and tuples.
- **Modules**: `import` and `from ... import` parse but raise. There is no module system, no standard library beyond the built-ins documented here, no third-party packages.
- **I/O**: `input()` returns an empty string in sandbox mode. There is no file system, no network, no `os`, no `sys`.
- **Threading and async beyond surface syntax**: `async def`, `await`, `async for`, `async with` parse and emit `MakeCoroutine`, but there is no event loop. They run as if synchronous.
- **Metaclasses, descriptors, decorators-on-classes, properties**: not modeled.
- **Dynamic code**: no `exec`, no `eval`, no `compile`, no `__import__`.
- **Reflection beyond `type`, `id`, `hash`, `repr`, `callable`, `getattr`, `hasattr`**.

## Why functional, not OO

Object-oriented features add a lot of surface and a lot of binary. Method resolution, inheritance chains, descriptor protocols, instance dictionaries, `__init__` / `__new__`, super, slots, and dunder dispatch all require runtime machinery that grows the VM and slows dispatch.

A functional core gives Edge Python:

- **A smaller binary**: the entire VM fits in ~130 KB of WebAssembly.
- **A faster interpreter**: no method resolution overhead. Built-ins are first-class `NativeFn` values; user functions are pure `(params, body, defaults, captures)` tuples.
- **Aggressive memoization**: pure functions get their results cached after two hits with the same arguments. Most functional code is pure by construction, so this catches a lot of the cost.
- **Easier sandboxing**: with no class system, the attack surface is the built-in set, which is fixed.

## Sandbox guarantees

When run with `--sandbox`, Edge Python enforces:

| Limit              | Default sandbox |
|--------------------|-----------------|
| Max operations     | 100,000,000     |
| Max heap bytes     | 100,000         |
| Max call depth     | 256             |

Hitting any limit raises a recoverable `RuntimeError` / `MemoryError` / `RecursionError` rather than crashing the host process. This matters when you embed the VM as a scripting layer.

## Where it runs

- **Native**: `x86_64-linux`, `aarch64-darwin`, `x86_64-windows`. Single binary, no runtime dependencies.
- **WebAssembly**: `wasm32-unknown-unknown`, `no_std`, `panic=abort`. Drops into any browser or wasm host.

The same compiler and the same VM run on both targets. There is no host-specific built-in.