/*
`parser.rs`
    Consumes the token stream from lexer.rs and emits bytecode directly. Single pass: no AST is built, opcodes are written as tokens are read.

----

mod modules {
    pub mod lexer;
    pub mod parser;
}

fn main() {

    /*
    Initialization point for the CDK.
    */

    initialize_logger();

    use modules::lexer::lexer;
    use modules::parser::Parser;

    let source = "name: str = 'Dylan'\nprint(f'Hey, I am: {name}.')";

    let chunk = Parser::new(source, lexer(source)).parse();

    // Instrucciones
    for (i, ins) in chunk.instructions.iter().enumerate() {
        info!("{:03} {:?} {}", i, ins.opcode, ins.operand);
    }

    let tokens: Vec<String> = modules::lexer::lexer(source)
        .map(|t| format!("{:?} [{}-{}]", t.kind, t.start, t.end))
        .collect();
    info!("{:?}", tokens);

    // Pools
    info!("constants: {:?}", chunk.constants);
    info!("names:     {:?}", chunk.names);

---

To do:

    Refactor the code to clean it up, update documentation, and review the testing system.

*/

use crate::modules::lexer::{Token, TokenType};
use std::iter::Peekable;

#[derive(Debug)] pub enum OpCode { LoadConst, LoadName, StoreName, Call, PopTop, ReturnValue, BuildString }
#[derive(Debug)] pub struct Instruction { pub opcode: OpCode, pub operand: u16 }
#[derive(Debug)] pub enum Value { Str(String), Int(i64), Float(f64), Bool(bool), None }

#[derive(Default)]
pub struct Chunk {
    pub instructions: Vec<Instruction>,
    pub constants:    Vec<Value>,
    pub names:        Vec<String>,
}

impl Chunk {
    fn emit(&mut self, op: OpCode, operand: u16) {
        self.instructions.push(Instruction { opcode: op, operand });
    }
    fn push_const(&mut self, v: Value) -> u16 {
        self.constants.push(v); (self.constants.len() - 1) as u16
    }
    fn push_name(&mut self, n: &str) -> u16 {
        if let Some(i) = self.names.iter().position(|x| x == n) { return i as u16; }
        self.names.push(n.to_string()); (self.names.len() - 1) as u16
    }
}

pub struct Parser<'src, I: Iterator<Item = Token>> {
    source: &'src str,
    tokens: Peekable<I>,
    chunk:  Chunk,
}

fn parse_string(s: &str) -> String {
    let is_raw = s.contains('r') || s.contains('R');
    let s = s.trim_start_matches(|c: char| "bBrRuU".contains(c));
    let inner = if s.starts_with("\"\"\"") || s.starts_with("'''") {
        &s[3..s.len() - 3]
    } else {
        &s[1..s.len() - 1]
    };
    if is_raw { inner.to_string() } else { unescape(inner) }
}

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' { out.push(c); continue; }
        match chars.next() {
            Some('n')  => out.push('\n'),
            Some('t')  => out.push('\t'),
            Some('r')  => out.push('\r'),
            Some('\\') => out.push('\\'),
            Some('\'') => out.push('\''),
            Some('"')  => out.push('"'),
            Some('0')  => out.push('\0'),
            Some(c)    => { out.push('\\'); out.push(c); }
            None       => out.push('\\'),
        }
    }
    out
}

impl<'src, I: Iterator<Item = Token>> Parser<'src, I> {
    pub fn new(source: &'src str, iter: I) -> Self {
        Self { source, tokens: iter.peekable(), chunk: Chunk::default() }
    }

    pub fn parse(mut self) -> Chunk {
        while !self.at_end() { self.expr(); self.chunk.emit(OpCode::PopTop, 0); }
        self.chunk.emit(OpCode::ReturnValue, 0);
        self.chunk
    }

    fn peek(&mut self) -> Option<TokenType> {
        loop {
            match self.tokens.peek().map(|t| t.kind.clone()) {
                Some(TokenType::Newline | TokenType::Nl | TokenType::Comment) => { self.tokens.next(); }
                Some(TokenType::Endmarker) | None => return None,
                Some(k) => return Some(k),
            }
        }
    }

    fn advance(&mut self) -> Token           { self.tokens.next().unwrap() }
    fn at_end(&mut self)   -> bool           { self.peek().is_none() }
    fn lexeme(&self, t: &Token) -> &'src str { &self.source[t.start..t.end] }

    fn expr(&mut self) {
        let t = self.advance();
        match t.kind {
            TokenType::Name         => self.name(t),
            TokenType::String => { self.emit_const(Value::Str(parse_string(self.lexeme(&t)))); },
            TokenType::Int => {
                let raw = self.lexeme(&t).replace('_', "");
                let v = if let Some(s) = raw.strip_prefix("0x").or(raw.strip_prefix("0X")) {
                    i64::from_str_radix(s, 16).unwrap_or(0)
                } else if let Some(s) = raw.strip_prefix("0o").or(raw.strip_prefix("0O")) {
                    i64::from_str_radix(s, 8).unwrap_or(0)
                } else if let Some(s) = raw.strip_prefix("0b").or(raw.strip_prefix("0B")) {
                    i64::from_str_radix(s, 2).unwrap_or(0)
                } else {
                    raw.parse().unwrap_or(0)
                };
                self.emit_const(Value::Int(v));
            },
            TokenType::Float => {
                let raw = self.lexeme(&t).replace('_', "");
                self.emit_const(Value::Float(raw.parse().unwrap_or(0.0)));
            },
            TokenType::True         => self.emit_const(Value::Bool(true)),
            TokenType::False        => self.emit_const(Value::Bool(false)),
            TokenType::None         => self.emit_const(Value::None),
            TokenType::FstringStart => self.fstring(),
            _                       => {}
        }
    }

    fn emit_const(&mut self, v: Value) {
        let idx = self.chunk.push_const(v);
        self.chunk.emit(OpCode::LoadConst, idx);
    }

    fn emit_name(&mut self, name: String) {
        let idx = self.chunk.push_name(&name);
        self.chunk.emit(OpCode::LoadName, idx);
    }

    fn name(&mut self, t: Token) {
        let name = self.lexeme(&t).to_string();
        if matches!(self.peek(), Some(TokenType::Colon)) { self.advance(); self.advance(); } // consume : tipo
        match self.peek() {
            Some(TokenType::Equal) => self.assign(name),
            Some(TokenType::Lpar)  => self.call(name),
            _                      => self.emit_name(name),
        }
    }

    fn assign(&mut self, name: String) {
        self.advance();                          // consume =
        self.expr();
        let idx = self.chunk.push_name(&name);
        self.chunk.emit(OpCode::StoreName, idx);
    }

    fn call(&mut self, name: String) {
        let idx = self.chunk.push_name(&name);
        self.chunk.emit(OpCode::LoadName, idx); // antes de los args
        self.advance();                          // consume (
        let mut argc = 0;
        while !matches!(self.peek(), Some(TokenType::Rpar) | None) {
            self.expr(); argc += 1;
            if matches!(self.peek(), Some(TokenType::Comma)) { self.advance(); }
        }
        self.advance();                          // consume )
        self.chunk.emit(OpCode::Call, argc);
    }

    fn fstring(&mut self) {
        let mut parts = 0u16;
        loop {
            match self.peek() {
                Some(TokenType::FstringMiddle) => {
                    let tok = self.advance();
                    let mut rest = self.lexeme(&tok);
                    while let Some(open) = rest.find('{') {
                        if open > 0 { self.emit_const(Value::Str(rest[..open].to_string())); parts += 1; }
                        rest = &rest[open + 1..];
                        if let Some(close) = rest.find('}') {
                            self.emit_name(rest[..close].trim().to_string()); parts += 1;
                            rest = &rest[close + 1..];
                        } else { break; }
                    }
                    if !rest.is_empty() { self.emit_const(Value::Str(rest.to_string())); parts += 1; }
                }
                Some(TokenType::FstringEnd) => { self.advance(); break; }
                _ => break,
            }
        }
        self.chunk.emit(OpCode::BuildString, parts);
    }
}