---
title: "Syntax"
description: "Formal specification of Edge Python syntax: bytecode emission, expression parsing, built-in dispatch, and f-string interpolation."
---

## Overview

Single pass parser: consumes the lexer token stream and emits bytecode directly. No abstract syntax tree is built — each construct is parsed and emitted in one traversal, keeping memory usage minimal.

## Bytecode Model

Each `Instruction` carries an `OpCode` and a `u16` operand. Constants are stored in `Chunk.constants`, names in `Chunk.names`. Both are referenced by index in the operand field.

| OpCode | Operand |
|---|---|
| `LoadConst` | Index into `constants` |
| `LoadName` | Index into `names` |
| `StoreName` | Index into `names` |
| `Call` | Argument count |
| `PopTop` | — |
| `ReturnValue` | — |
| `BuildString` | Part count |
| `FormatValue` | — |
| `Minus` | — |
| `CallPrint` | Argument count |
| `CallLen` | Argument count |
| `CallAbs` | Argument count |
| `CallStr` | Argument count |
| `CallInt` | Argument count |
| `CallRange` | Always `3` |

## Expression Parsing

`expr()` advances one token and dispatches on its kind. Every expression leaves exactly one value on the stack. Unrecognized tokens are silently skipped.

Supported expression types: `Name`, `String`, `Int`, `Float`, `True`, `False`, `None`, `FstringStart`, `Minus`.

## Assignment and Type Annotations

Type annotations (`name: type = value`) are supported syntactically — the colon and type name are consumed and discarded. Only the value is emitted.

```python
value: int = 42   # annotation discarded, emits LoadConst + StoreName
x = 42            # same bytecode
```

## Built-in Dispatch

Built-in calls are resolved at parse time by name and emitted as dedicated opcodes, bypassing the general `Call` path:
```
print → CallPrint
len   → CallLen
abs   → CallAbs
str   → CallStr
int   → CallInt
range → CallRange
_     → LoadName + Call
```

## Range Normalization

`range` arguments are normalized at compile time. The VM always receives exactly 3 `LoadConst` values followed by `CallRange 3`, regardless of how many arguments were passed.

| Source | Emitted |
|---|---|
| `range(n)` | `0, n, 1` |
| `range(a, b)` | `a, b, 1` |
| `range(a, b, c)` | `a, b, c` |

## F-String Interpolation

F-strings are parsed from the `FstringStart → FstringMiddle → FstringEnd` token sequence produced by the lexer. Each `FstringMiddle` is scanned for `{name}` expressions. Literal segments emit `LoadConst`, interpolated names emit `LoadName + FormatValue`. All parts are combined with `BuildString N`.
```python
f"Hey, {name}."
# LoadConst "Hey, "
# LoadName  name
# FormatValue
# LoadConst "."
# BuildString 3
```

Current coverage: simple name interpolation `{name}`. Expressions `{1 + 2}`, format specs `{x:.2f}`, and conversion flags `{x!r}` are not yet supported.

## Integration Tests

Tests live in `parser_test.rs` and load cases from `cases/parser_cases.json`. Each case is a tuple of `[source, expected_constants, expected_names, expected_instructions]`. `test_cases` compiles each source and asserts constants, names, and bytecode sequence match exactly.

## Module Usage and Output
```rust
`parser.rs`
    Single pass: consumes lexer tokens and emits bytecode directly. No abstract syntax tree built, fast and minimal memory.

    Usage:
        ```rust
        mod modules {
            pub mod lexer;
            pub mod parser;
        }

        let source = "value: int = abs(-42)";

        let chunk = modules::parser::Parser::new(source, modules::lexer::lexer(source)).parse();

        // Instructions.
        for (i, ins) in chunk.instructions.iter().enumerate() {
            info!("{:03} {:?} {}", i, ins.opcode, ins.operand);
        }

        let tokens: Vec<String> = modules::lexer::lexer(source)
            .map(|t| format!("{:?} [{}-{}]", t.kind, t.start, t.end))
            .collect();

        info!("{:?}", tokens);

        info!("constants: {:?}", chunk.constants);
        info!("names: {:?}", chunk.names);
        ```

    Output:
        ```bash
        2026-03-18T05:42:07.381Z INFO  [compiler] 000 LoadConst 0
        2026-03-18T05:42:07.381Z INFO  [compiler] 001 Minus 0
        2026-03-18T05:42:07.381Z INFO  [compiler] 002 CallAbs 1
        2026-03-18T05:42:07.381Z INFO  [compiler] 003 StoreName 0
        2026-03-18T05:42:07.381Z INFO  [compiler] 004 PopTop 0
        2026-03-18T05:42:07.381Z INFO  [compiler] 005 ReturnValue 0
        2026-03-18T05:42:07.381Z INFO  [compiler] ["Name [0-5]", "Colon [5-6]", "Name [7-10]", "Equal [11-12]", "Name [13-16]", "Lpar [16-17]", "Minus [17-18]", "Int [18-20]", "Rpar [20-21]", "Endmarker [20-21]"]
        2026-03-18T05:42:07.381Z INFO  [compiler] constants: [Int(42)]
        2026-03-18T05:42:07.381Z INFO  [compiler] names: ["value"]
        ```
```