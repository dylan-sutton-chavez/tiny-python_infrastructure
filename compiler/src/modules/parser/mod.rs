// parser/mod.rs

pub(super) mod types;

mod stmt;
mod control;
mod expr;
mod literals;

pub use types::*;

use crate::s;
use crate::modules::lexer::{Token, TokenType};
use crate::modules::fx::FxHashMap as HashMap;

use alloc::{string::{String, ToString}, vec::Vec};
use core::iter::Peekable;

/* Main parser state holding source, tokens, SSA chunk, versions and control stacks. */

pub struct Parser<'src, I: Iterator<Item = Token>> {
    pub(super) source: &'src str,
    pub(super) tokens: Peekable<I>,
    pub(super) chunk: SSAChunk,
    pub(super) ssa_versions: HashMap<String, u32>,
    pub(super) join_stack: Vec<JoinNode>,
    pub(super) loop_starts: Vec<u16>,
    pub(super) last_line: usize,
    pub(super) loop_breaks: Vec<Vec<usize>>,
    pub(super) expr_depth: usize,
    pub(super) saw_newline: bool,
    pub errors: Vec<Diagnostic>,
}

/* Tracks and updates SSA versions for variables to enable static single assignment. */

impl<'src, I: Iterator<Item = Token>> Parser<'src, I> {
    pub(super) fn current_version(&self, name: &str) -> u32 {
        self.ssa_versions.get(name).copied().unwrap_or(0)
    }

    pub(super) fn ssa_name<'a>(name: &str, ver: u32, buf: &'a mut [u8; 128]) -> &'a str {
        let name_bytes = name.as_bytes();
        let cap = buf.len();
        let mut n = name_bytes.len().min(cap);
        buf[..n].copy_from_slice(&name_bytes[..n]);
        if n < cap {
            buf[n] = b'_';
            n += 1;
        }
        let mut tmp = itoa::Buffer::new();
        let s = tmp.format(ver).as_bytes();
        let take = s.len().min(cap - n);
        buf[n..n + take].copy_from_slice(&s[..take]);
        n += take;
        unsafe { core::str::from_utf8_unchecked(&buf[..n]) }
    }

    pub(super) fn increment_version(&mut self, name: &str) -> u32 {
        let cur = self.current_version(name);
        let new = cur + 1;
        self.ssa_versions.insert(name.to_string(), new);
        new
    }

    pub(super) fn push_ssa_name(&mut self, name: &str, ver: u32) -> u16 {
        let mut buf = [0u8; 128];
        self.chunk.push_name(Self::ssa_name(name, ver, &mut buf))
    }

    pub(super) fn emit_load_ssa(&mut self, name: String) {
        let v = self.current_version(&name);
        let mut buf = [0u8; 128];
        let i = self.chunk.push_name(Self::ssa_name(&name, v, &mut buf));
        self.chunk.emit(OpCode::LoadName, i);
    }

    pub(super) fn emit_const(&mut self, v: Value) {
        let i = self.chunk.push_const(v);
        self.chunk.emit(OpCode::LoadConst, i);
    }

    pub(super) fn store_name(&mut self, name: String) {
        let ver = self.increment_version(&name);
        let mut buf = [0u8; 128];
        let i = self.chunk.push_name(Self::ssa_name(&name, ver, &mut buf));
        self.chunk.emit(OpCode::StoreName, i);
    }

    pub(super) fn with_fresh_chunk(&mut self, f: impl FnOnce(&mut Self)) -> SSAChunk {
        let saved_chunk = core::mem::take(&mut self.chunk);
        let saved_ver = self.ssa_versions.clone();
        f(self);
        let body = core::mem::take(&mut self.chunk);
        self.chunk = saved_chunk;
        self.ssa_versions = saved_ver;
        body
    }
}

/* Handles SSA merging for if/else blocks and creates PHI nodes at control-flow joins. */

impl<'src, I: Iterator<Item = Token>> Parser<'src, I> {
    pub(super) fn enter_block(&mut self) {
        self.join_stack.push(JoinNode {
            backup: self.ssa_versions.clone(),
            then: None,
        });
    }

    pub(super) fn mid_block(&mut self) {
        let Some(j) = self.join_stack.last_mut() else { return };
        j.then = Some(self.ssa_versions.clone()); // snapshot then-branch before overwriting with else baseline
        let mut restored = j.backup.clone();
        for (name, &v) in &self.ssa_versions {
            let e = restored.entry(name.clone()).or_insert(0);
            *e = (*e).max(v);
        }
        self.ssa_versions = restored;
    }

    pub(super) fn commit_block(&mut self) {
        let Some(j) = self.join_stack.pop() else { return };
        let post = self.ssa_versions.clone();

        let (a, b) = match j.then {
            Some(t) => (t, post),
            None => (post, j.backup.clone()),
        };

        let mut divergent: Vec<&String> = a
            .keys()
            .chain(b.keys())
            .filter(|name| a.get(*name).unwrap_or(&0) != b.get(*name).unwrap_or(&0))
            .collect();
        divergent.sort(); // deterministic Phi order regardless of HashMap iteration
        divergent.dedup(); // chain() can produce duplicates when both branches define the same var

        for name in divergent {
            let va = *a.get(name).unwrap_or(&0);
            let vb = *b.get(name).unwrap_or(&0);
            let mut ba = [0u8; 128];
            let mut bb = [0u8; 128];
            let mut bx = [0u8; 128];
            let ia = self.chunk.push_name(Self::ssa_name(name, va, &mut ba));
            let ib = self.chunk.push_name(Self::ssa_name(name, vb, &mut bb));
            let v = self.increment_version(name);
            let ix = self.chunk.push_name(Self::ssa_name(name, v, &mut bx));

            self.chunk.phi_sources.push((ia, ib));
            self.chunk.emit(OpCode::Phi, ix);
        }
    }
}

/* Utility methods to advance, peek, eat tokens and report parser errors cleanly. */

impl<'src, I: Iterator<Item = Token>> Parser<'src, I> {
    pub(super) fn advance(&mut self) -> Token {
        let tok = self.tokens.next().unwrap_or(Token {
            kind: TokenType::Endmarker,
            line: 0, start: 0, end: 0,
        });
        self.last_line = tok.line;
        tok
    }

    pub(super) fn error(&mut self, msg: &str) {
        let (line, byte_offset, end) = self
            .tokens
            .peek()
            .map(|t| (t.line, t.start, t.end))
            .unwrap_or((self.last_line, 0, 0));

        // Look for the last '\n' before the token.
        let col = self.source[..byte_offset]
            .rfind('\n')
            .map(|line_start| byte_offset - line_start - 1)
            .unwrap_or(byte_offset);

        self.errors.push(Diagnostic { line, col, end, msg: msg.to_string() });
        loop {
            match self.tokens.peek().map(|t| t.kind) {
                None | Some(TokenType::Newline | TokenType::Dedent | TokenType::Endmarker) => break,
                _ => { self.tokens.next(); }
            }
        }
    }

    pub(super) fn at_end(&mut self) -> bool { self.peek().is_none() }

    pub(super) fn lexeme(&self, t: &Token) -> &'src str { &self.source[t.start..t.end] }

    pub(super) fn peek(&mut self) -> Option<TokenType> {
        loop {
            match self.tokens.peek().map(|t| t.kind) {
                Some(TokenType::Newline) => {
                    self.saw_newline = true;
                    self.tokens.next();
                }
                Some(TokenType::Nl | TokenType::Comment) => { self.tokens.next(); }
                Some(k) => return Some(k),
                None => return None,
            }
        }
    }

    pub(super) fn patch(&mut self, pos: usize) {
        self.chunk.instructions[pos].operand = self.chunk.instructions.len() as u16;
    }

    pub(super) fn eat(&mut self, kind: TokenType) {
        if matches!(self.peek(), Some(k) if k == kind) {
            self.advance();
        } else {
            let token_text = match self.tokens.peek() {
                Some(t) => &self.source[t.start..t.end],
                None => "EOF",
            };

            // Query the lexer's static lookup table for the expected token's name.
            let label = kind.as_str();

            self.error(&s!("expected ", str label, ", got '", str token_text, "'"));
        }
    }

    pub(super) fn eat_if(&mut self, kind: TokenType) -> bool {
        if matches!(self.peek(), Some(k) if k == kind) {
            self.advance();
            true
        } else {
            false
        }
    }
}

/* Parser constructor and main parse method that drives full compilation to SSA. */

impl<'src, I: Iterator<Item = Token>> Parser<'src, I> {
    pub fn new(source: &'src str, iter: I) -> Self {
        Self {
            source,
            tokens: iter.peekable(),
            chunk: SSAChunk::default(),
            ssa_versions: HashMap::default(),
            join_stack: Vec::new(),
            loop_starts: Vec::new(),
            loop_breaks: Vec::new(),
            saw_newline: false,
            expr_depth: 0,
            last_line: 0,
            errors: Vec::new(),
        }
    }

    pub fn parse(mut self) -> (SSAChunk, Vec<Diagnostic>) {
        while !self.at_end() {
            while self.eat_if(TokenType::Semi) {}
            if self.at_end() { break; }

            let produced_value = self.stmt();
            if !self.at_end() && produced_value { self.chunk.emit(OpCode::PopTop, 0); }
        }

        if self.chunk.overflow {
            let line = self.errors.last().map(|e| e.line).unwrap_or(0);
            self.errors.push(Diagnostic {
                line, col: 0, end: 0,
                msg: "program too large: exceeded maximum instruction limit".to_string()
            });
        }

        if !self.errors.is_empty() {
            self.chunk.instructions.clear();
            self.chunk.constants.clear();
            self.chunk.names.clear();
        }

        self.chunk.emit(OpCode::ReturnValue, 0);
        self.chunk.finalize_prev_slots();
        (self.chunk, self.errors)
    }
}