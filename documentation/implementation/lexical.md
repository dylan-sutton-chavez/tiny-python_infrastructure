---
title: "Lexical"
description: "Tokenization, indentation, f-strings, and source-level limits."
---

## Overview

Edge Python uses a hand-written, LUT-driven scanner that walks the source as raw bytes and produces a stream of `Token { kind, line, start, end }`. The scanner is offset-based: tokens carry byte indices into the source buffer, never their own copies of the text. This is enough for diagnostics, debug output, and the parser's lazy slicing of identifier and string content.

Lexing runs in linear time *O(n)* with constant-time per-byte branchless dispatch through two lookup tables.

## Token kinds

The token set tracks Python 3.13.12 closely. Categories implemented:

- **Keywords**: `False`, `None`, `True`, `and`, `as`, `assert`, `async`, `await`, `break`, `class`, `continue`, `def`, `del`, `elif`, `else`, `except`, `finally`, `for`, `from`, `global`, `if`, `import`, `in`, `is`, `lambda`, `nonlocal`, `not`, `or`, `pass`, `raise`, `return`, `try`, `while`, `with`, `yield`.
- **Soft keywords**: `case`, `match`, `type`, `_`. Resolved contextually (see below).
- **Operators**: 1-, 2-, and 3-character operator forms (`+`, `==`, `**=`, `//=`, etc.).
- **Delimiters**: `( ) [ ] { } : , ; .`.
- **Literals**: `Name`, `Int`, `Float`, `Complex`, `String`.
- **F-string segments**: `FstringStart`, `FstringMiddle`, `FstringEnd`.
- **Whitespace and structure**: `Comment`, `Newline`, `Indent`, `Dedent`, `Nl`, `Endmarker`.

## Dispatch tables

The lexer hot loop avoids per-byte branching through two compile-time tables in `lexer/tables.rs`:

```rust
// Bit flags per byte: ID_START, ID_CONT, DIGIT, SPACE.
// Indexed lookup replaces a chain of comparisons.
pub static BYTE_CLASS: [u8; 256] = { /* ... */ };

// Single-char operator dispatch: byte -> small index -> TokenType.
pub static SINGLE_TOK: [u8; 128] = { /* ... */ };
pub const SINGLE_MAP: [TokenType; 24] = { /* ... */ };
```

Identifiers, digits, and whitespace are scanned with a `scan_while(pred)` driver that loops over `BYTE_CLASS[b] & FLAG`. Single-character operators do `b → SINGLE_TOK[b] → SINGLE_MAP[i]` — two indexed loads, no branches.

The keyword lookup is routed by `(length, first_byte)` to skip most `memcmp`s. Most keyword candidates terminate after a single match arm.

## Numeric literals

```python
42
1_000_000        # underscore separators
0xDEAD_BEEF      # hex
0o777            # octal
0b1010_1010      # binary
3.14
.5               # leading-dot float
1e-5             # exponent
3j               # complex (lexed; only real part survives at runtime)
```

The number scanner handles base prefixes, underscore separators, optional exponents, the leading-dot form, and the trailing `j` / `J` for complex literals.

## String prefixes

```python
'plain'          # str
b'bytes'         # bytes (lexed as String)
r'raw\n'         # raw
u'unicode'       # unicode
br'rawbytes'     # raw bytes
RB'mixed'        # any case combination
f'fstring'       # f-string (separate token sequence)
fr'raw fstring'  # raw f-string
"""triple"""     # triple-quoted, single or double
```

A leading prefix is recognized before the opening quote by the identifier scanner and verified against `is_string_prefix` / `is_fstring_prefix`. Triple-quoted strings span newlines and bump `line` for each `\n` inside.

## F-strings

F-strings are decomposed into a sequence of tokens rather than being represented by a single `String` token. The parser consumes the sequence directly:

```text
f'a {x} b {y + 1}!'
   │
   ▼
FstringStart
FstringMiddle("a ")
Lbrace
Name(x)
Rbrace
FstringMiddle(" b ")
Lbrace
Name(y) Plus Int(1)
Rbrace
FstringMiddle("!")
FstringEnd
```

Expression tokens between `{` and `}` are emitted by the **main lexer**, not the f-string scanner, which means the full expression grammar is available inside interpolations without special casing.

`{{` and `}}` are treated as escaped literal braces and produce no `Lbrace` / `Rbrace`. They survive into the `FstringMiddle` text and are unescaped by the parser.

Triple-quoted f-strings (`f"""..."""`) follow the same structure with newlines embedded in the middle segments.

## Indentation

Edge Python uses CPython's INDENT/DEDENT model. The scanner tracks a stack of column counts and emits structural tokens at line boundaries:

| Situation                         | Tokens emitted                       |
|-----------------------------------|--------------------------------------|
| Blank line or comment-only line   | `Nl`                                 |
| Inside `(...)`, `[...]`, `{...}`  | `Nl` (no INDENT/DEDENT)              |
| Indentation increased             | `Newline`, `Indent`                  |
| Indentation decreased             | `Newline`, `Dedent` (× n levels)     |
| Indentation unchanged             | `Newline`                            |
| Mixed tabs and spaces in indent   | `Endmarker` (lex halt)               |

The `nesting` counter is bumped by `(`, `[`, `{` and decremented by `)`, `]`. While `nesting > 0`, line breaks emit `Nl` and the indent stack is frozen — this is what allows multi-line expressions inside brackets without spurious INDENT/DEDENT.

## Soft-keyword disambiguation

`match`, `case`, and `type` are keywords in some positions and identifiers in others. The lexer resolves the ambiguity by peeking at the *next* token:

```python
match x:           # 'match' is a keyword
    case 1: ...

match = 5          # 'match' is an identifier
case = True        # 'case' is an identifier
type X = int       # 'type' is a keyword (alias declaration)
type = None        # 'type' is an identifier
```

If the token following `match` / `case` / `type` is one of `(`, `)`, `]`, `:`, `=`, `,`, `Newline`, or `EOF`, the soft keyword is downgraded to `Name`. Otherwise it stays a keyword.

The same logic applies to `_`. In `case _:` it's the wildcard `Underscore`; in `_ = compute()` it's a `Name`.

## Comments

`#` to end-of-line. Comments are emitted as a `Comment` token rather than discarded — this is what allows tools to round-trip source. The parser ignores `Comment` and `Nl` tokens during `peek()`.

## Limits

To prevent asymmetric DoS attacks (small input that exhausts memory or time), the lexer enforces hard caps. Going past any of these halts lexing with `Endmarker`:

| Constant            | Value     | Purpose                                  |
|---------------------|-----------|------------------------------------------|
| `MAX_SOURCE_SIZE`   | 10 MiB    | Reject oversized input upfront           |
| `MAX_INDENT_DEPTH`  | 100       | Cap on the indentation stack             |
| `MAX_FSTRING_DEPTH` | 200       | Cap on nested f-string contexts          |

These match the OWASP A04:2021 advice on resource exhaustion in interpreters.

## Why offset-based tokens

A `Token` is 32 bytes:

```rust
pub struct Token {
    pub kind: TokenType,   // 1 byte + padding
    pub line: usize,
    pub start: usize,
    pub end: usize,
}
```

The parser slices `&source[t.start..t.end]` lazily when it needs the lexeme — for identifier names, string content, or numeric literals. This means:

- The lexer never allocates a `String` per identifier.
- The parser's `lexeme(&t)` is a zero-copy `&str` that lives as long as the source buffer.
- Diagnostics get exact byte offsets for free, which makes error column computation a single `rfind('\n')`.

## References

- Python language reference, *Lexical analysis*: docs.python.org/3/reference/lexical_analysis
- OWASP, *Insecure Compiler Optimization*: owasp.org/www-community/vulnerabilities/Insecure_Compiler_Optimization
- Aho, Sethi & Ullman. *Compilers: Principles, Techniques and Tools* (1986). LUT-driven scanners.