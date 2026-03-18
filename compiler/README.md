*Update this documentation upon completion of the compiler (https://edgepython.com/resources/architecture)*

## Project Tree

```bash
├── Cargo.toml
├── README.md
├── src
│   ├── lib.rs
│   ├── main.rs
│   └── modules
│       ├── compiler.rs
│       ├── lexer.rs
│       ├── opcodes.rs
│       ├── parser.rs
│       └── vm.rs
└── tests
```

```bash
compiler.rs
  n/A

lexer.rs
  Tokenizes Python source into a stream of spanned Token variants.

opcodes.rs
  n/A

parser.rs
  Single pass: consumes lexer tokens and emits bytecode directly. No abstract syntax tree built, fast and minimal memory.

vm.rs
  n/A
```

*upx packer*