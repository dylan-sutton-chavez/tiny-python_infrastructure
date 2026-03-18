/*
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
*/

use crate::modules::lexer::{Token, TokenType};
use std::iter::Peekable;

#[derive(Debug)]
pub enum OpCode {
    LoadConst, LoadName, StoreName, Call, PopTop, ReturnValue,
    BuildString, CallPrint, CallLen, FormatValue, CallAbs, Minus,
    CallStr, CallInt, CallRange
}

#[derive(Debug)] pub struct Instruction { pub opcode: OpCode, pub operand: u16 }
#[derive(Debug)] pub enum Value { Str(String), Int(i64), Float(f64), Bool(bool), None, Range(i64, i64, i64) }

#[derive(Default)]
pub struct Chunk {
    pub instructions: Vec<Instruction>,
    pub constants: Vec<Value>,
    pub names: Vec<String>,
}

impl Chunk {
    fn emit(&mut self, op: OpCode, operand: u16) { self.instructions.push(Instruction { opcode: op, operand }); }
    fn push_const(&mut self, v: Value) -> u16 { self.constants.push(v); (self.constants.len()-1) as u16 }
    fn push_name(&mut self, n: &str) -> u16 {
        if let Some(i) = self.names.iter().position(|x| x == n) { return i as u16; }
        self.names.push(n.to_string()); (self.names.len()-1) as u16
    }
}

pub struct Parser<'src, I: Iterator<Item = Token>> {
    source: &'src str,
    tokens: Peekable<I>,
    chunk: Chunk,
}

fn parse_string(s: &str) -> String {
    let is_raw = s.contains('r') || s.contains('R');
    let s = s.trim_start_matches(|c: char| "bBrRuU".contains(c));
    let inner = if s.starts_with("\"\"\"") || s.starts_with("'''") { &s[3..s.len()-3] } else { &s[1..s.len()-1] };
    if is_raw { inner.to_string() } else { unescape(inner) }
}

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' { out.push(c); continue; }
        match chars.next() {
            Some('n') => out.push('\n'), Some('t') => out.push('\t'),
            Some('r') => out.push('\r'), Some('\\') => out.push('\\'),
            Some('\'') => out.push('\''), Some('"') => out.push('"'),
            Some('0') => out.push('\0'), Some(c) => { out.push('\\'); out.push(c); }
            None => out.push('\\'),
        }
    }
    out
}

impl<'src, I: Iterator<Item = Token>> Parser<'src, I> {
    pub fn new(source: &'src str, iter: I) -> Self { Self { source, tokens: iter.peekable(), chunk: Chunk::default() } }

    pub fn parse(mut self) -> Chunk {
        while !self.at_end() { self.expr(); if !self.at_end() { self.chunk.emit(OpCode::PopTop, 0); } }
        self.chunk.emit(OpCode::ReturnValue, 0);
        self.chunk
    }

    fn peek(&mut self) -> Option<TokenType> {
        loop {
            match self.tokens.peek().map(|t| t.kind.clone()) {
                Some(TokenType::Newline | TokenType::Nl | TokenType::Comment) => { self.tokens.next(); }
                Some(k) => return Some(k),
                None => return None,
            }
        }
    }

    fn advance(&mut self) -> Token { self.tokens.next().unwrap() }
    fn at_end(&mut self) -> bool { self.peek().is_none() }
    fn lexeme(&self, t: &Token) -> &'src str { &self.source[t.start..t.end] }

    fn expr(&mut self) {
        let t = self.advance();
        match t.kind {
            TokenType::Name => self.name(t),
            TokenType::String => self.emit_const(Value::Str(parse_string(self.lexeme(&t)))),
            TokenType::Int => self.parse_int(self.lexeme(&t)),
            TokenType::Float => self.parse_float(self.lexeme(&t)),
            TokenType::True => self.emit_const(Value::Bool(true)),
            TokenType::False => self.emit_const(Value::Bool(false)),
            TokenType::None => self.emit_const(Value::None),
            TokenType::FstringStart => self.fstring(),
            TokenType::Minus => self.minus(),
            _ => {}
        }
    }

    fn minus(&mut self) {
        self.expr();
        self.chunk.emit(OpCode::Minus, 0);
    }

    fn parse_int(&mut self, raw: &str) {
        let s = raw.replace('_', "");
        let v = if let Some(s) = s.strip_prefix("0x").or(s.strip_prefix("0X")) { i64::from_str_radix(s, 16).unwrap_or(0) }
        else if let Some(s) = s.strip_prefix("0o").or(s.strip_prefix("0O")) { i64::from_str_radix(s, 8).unwrap_or(0) }
        else if let Some(s) = s.strip_prefix("0b").or(s.strip_prefix("0B")) { i64::from_str_radix(s, 2).unwrap_or(0) }
        else { s.parse().unwrap_or(0) };
        self.emit_const(Value::Int(v));
    }

    fn parse_float(&mut self, raw: &str) {
        let s = raw.replace('_', "");
        self.emit_const(Value::Float(s.parse().unwrap_or(0.0)));
    }

    fn emit_const(&mut self, v: Value) {
        let i = self.chunk.push_const(v);
        self.chunk.emit(OpCode::LoadConst, i);
    }

    fn emit_name(&mut self, name: String) {
        let i = self.chunk.push_name(&name);
        self.chunk.emit(OpCode::LoadName, i);
    }

    fn name(&mut self, t: Token) {
        let name = self.lexeme(&t).to_string();
        if matches!(self.peek(), Some(TokenType::Colon)) { self.advance(); self.advance(); }
        match self.peek() {
            Some(TokenType::Equal) => self.assign(name),
            Some(TokenType::Lpar)  => self.call(name),
            _ => self.emit_name(name),
        }
    }

    fn assign(&mut self, name: String) {
        self.advance();
        self.expr();
        let i = self.chunk.push_name(&name);
        self.chunk.emit(OpCode::StoreName, i);
    }

    fn call(&mut self, name: String) {
        match name.as_str() {
            "print" => self.call_builtin(OpCode::CallPrint),
            "len" => self.call_builtin(OpCode::CallLen),
            "abs" => self.call_builtin(OpCode::CallAbs),
            "str" => self.call_builtin(OpCode::CallStr),
            "int" => self.call_builtin(OpCode::CallInt),
            "range" => self.call_range(),
            _ => self.call_normal(name)
        }
    }

    fn call_range(&mut self) {
        self.advance();
        let mut args = Vec::new();
        while !matches!(self.peek(), Some(TokenType::Rpar) | None) {
            let token = self.advance();
            if let TokenType::Int = token.kind {
                args.push(self.lexeme(&token).replace('_', "").parse::<i64>().unwrap_or(0));
            }
            if matches!(self.peek(), Some(TokenType::Comma)) { self.advance(); }
        }
        self.advance();

        let (start, stop, step) = match args.as_slice() {
            [stop]              => (0,      *stop,  1),
            [start, stop]       => (*start, *stop,  1),
            [start, stop, step] => (*start, *stop, *step),
            _                   => (0,      0,      1),
        };

        for v in [start, stop, step] { self.emit_const(Value::Int(v)); }
        self.chunk.emit(OpCode::CallRange, 3);
    }

    fn call_builtin(&mut self, op: OpCode) {
        self.advance();
        let mut argc = 0;
        while !matches!(self.peek(), Some(TokenType::Rpar) | None) {
            self.expr(); argc += 1;
            if matches!(self.peek(), Some(TokenType::Comma)) { self.advance(); }
        }
        self.advance();
        self.chunk.emit(op, argc);
    }

    fn call_normal(&mut self, name: String) {
        let i = self.chunk.push_name(&name);
        self.chunk.emit(OpCode::LoadName, i);
        self.advance();
        let mut argc = 0;
        while !matches!(self.peek(), Some(TokenType::Rpar) | None) {
            self.expr(); argc += 1;
            if matches!(self.peek(), Some(TokenType::Comma)) { self.advance(); }
        }
        self.advance();
        self.chunk.emit(OpCode::Call, argc);
    }

    fn fstring(&mut self) {
        let mut parts = 0u16;
        loop {
            match self.peek() {
                Some(TokenType::FstringMiddle) => {
                    let t = self.advance();
                    let mut rest = self.lexeme(&t);

                    while let Some(open) = rest.find('{') {
                        if open > 0 {
                            self.emit_const(Value::Str(rest[..open].to_string()));
                            parts += 1;
                        }
                        rest = &rest[open + 1..];

                        if let Some(close) = rest.find('}') {
                            let expr = rest[..close].trim();
                            if !expr.is_empty() {
                                self.emit_name(expr.to_string());
                                self.chunk.emit(OpCode::FormatValue, 0);
                                parts += 1;
                            }
                            rest = &rest[close + 1..];
                        } else {
                            break;
                        }
                    }

                    if !rest.is_empty() {
                        self.emit_const(Value::Str(rest.to_string()));
                        parts += 1;
                    }
                }
                Some(TokenType::FstringEnd) => { self.advance(); break; }
                _ => break,
            }
        }
        if parts > 0 {
            self.chunk.emit(OpCode::BuildString, parts);
        }
    }
}