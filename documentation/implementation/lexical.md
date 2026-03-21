---
title: "Lexical"
description: "Lexical grammar and tokenization model."
---

## Overview

Implements offset based token representation with linear time *O(n)* lexing. The lexer produces a stream of spanned tokens referencing byte offsets into the source buffer, eliminating string allocation overhead while preserving full position information for error reporting and debugging.

## Tokens Definition

The token set is derived from Python 3.13.12 `keyword` and `token` modules, with subset implementation in `lexer.rs`. 

* Types
* Keywords
* Builtin
* Lexical

```python
from keyword import kwlist, softkwlist # Keywords and soft seywords.
from token import EXACT_TOKEN_TYPES, tok_name # Operators, delimitors and token names.
```

## Indentation

The lexer emits `Nl` for blank lines, comments, and lines inside brackets. For all other lines, it emits `Newline` followed by `Indent`, `Dedent`, or nothing depending on whether indentation increased, decreased, or stayed the same. Mixing spaces and tabs halts the lexer with `Endmarker`.

## F-Strings

F-strings are decomposed into a token sequence rather than a single String token: `FstringStart → FstringMiddle → Lbrace → (expr tokens) → Rbrace → FstringEnd`. Tooling that consumes the token stream should handle each segment explicitly.

Expression tokens inside `{}` are emitted by the main lexer — not the f-string scanner — allowing full expression support without special-casing.

`{{` and `}}` are treated as escaped literal braces and do not produce `Lbrace`/`Rbrace` tokens.

## Soft Keyword Disambiguation

`match`, `case`, and `type` can also be used as identifiers. The lexer resolves the ambiguity by checking what follows: if the next token is `(` `)` `]` `:` `=` `,` `Newline` or `EOF`, they are emitted as `Name`.

## Lexer Security

Based on OWASP standards, the 04:2021 was adapted to prevent asymmetric DoS attacks, using limiters:

| constant            | value |
|---------------------|-------|
| MAX_INDENT_DEPTH    |  100  |
| MAX_FSTRING_DEPTH   |  200  |

## References
 
- Python language reference: docs.python.org/3/reference/lexical_analysis
- OWASP lexical attack prevention: owasp.org/www-community/vulnerabilities/Insecure_Compiler_Optimization
- Formally verified linear-time lexing: 2510.18479