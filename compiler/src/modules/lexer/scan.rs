// lexer/scan.rs

use super::TokenType;
use super::tables::*;
use alloc::vec::Vec;
use core::cmp::Ordering;

const MAX_INDENT_DEPTH: usize = 100;
const MAX_FSTRING_DEPTH: usize = 200;

/*
Scanner State
    Source bytes, position, pending queue, indent stack and f-string context.
*/

pub(super) struct Scanner<'a> {
    pub src: &'a [u8],
    pub pos: usize,
    pub pending: Vec<(TokenType,usize,usize,usize)>,
    pub indent_stack: Vec<usize>,
    pub nesting: u32,
    pub line: usize,
    pub fstring_stack: Vec<(u8, bool, usize, u32)>
}

impl<'a> Scanner<'a> {
    pub fn new(src: &'a [u8]) -> Self {
        Self {
            src, pos: 0,
            pending: Vec::new(),
            indent_stack: Vec::new(),
            nesting: 0, line: 0,
            fstring_stack: Vec::new()
        }
    }

    /*
    Hot-Path Helpers
        All inner loops use BYTE_CLASS indexed loads, zero branches per byte.
    */

    #[inline]
    pub fn skip_whitespace(&mut self) {
        while self.pos < self.src.len()
            && BYTE_CLASS[self.src[self.pos] as usize] & SPACE != 0
        { self.pos += 1; }
    }

    fn scan_id_rest(&mut self) {
        while self.pos < self.src.len()
            && BYTE_CLASS[self.src[self.pos] as usize] & ID_CONT != 0
        { self.pos += 1; }
    }

    fn scan_digits(&mut self) {
        while self.pos < self.src.len() {
            let b = self.src[self.pos];
            if BYTE_CLASS[b as usize] & DIGIT != 0 || b == b'_' { self.pos += 1; }
            else { break; }
        }
    }

    fn scan_hex_digits(&mut self) {
        while self.pos < self.src.len() {
            let b = self.src[self.pos];
            if b.is_ascii_hexdigit() || b == b'_' { self.pos += 1; } else { break; }
        }
    }

    fn scan_oct_digits(&mut self) {
        while self.pos < self.src.len() {
            match self.src[self.pos] {
                b'0'..=b'7' | b'_' => self.pos += 1,
                _ => break,
            }
        }
    }

    fn scan_bin_digits(&mut self) {
        while self.pos < self.src.len() {
            match self.src[self.pos] {
                b'0' | b'1' | b'_' => self.pos += 1,
                _ => break,
            }
        }
    }

    #[inline(always)]
    fn at(&self, offset: usize) -> Option<u8> {
        self.src.get(self.pos + offset).copied()
    }

    /*
    Number Scanning
        Handles decimal, hex, octal, binary, float and complex number literals.
    */

    fn scan_number(&mut self, start: usize) -> TokenType {
        if self.src[start] == b'0' && self.pos < self.src.len() {
            match self.src[self.pos] {
                b'x' | b'X' => { self.pos += 1; self.scan_hex_digits(); return self.maybe_complex(TokenType::Int); }
                b'o' | b'O' => { self.pos += 1; self.scan_oct_digits(); return self.maybe_complex(TokenType::Int); }
                b'b' | b'B' => { self.pos += 1; self.scan_bin_digits(); return self.maybe_complex(TokenType::Int); }
                _ => {}
            }
        }
        self.scan_digits();
        let mut is_float = false;
        if self.pos < self.src.len() && self.src[self.pos] == b'.'
            && self.at(1) != Some(b'.')
        {
            is_float = true;
            self.pos += 1;
            self.scan_digits();
        }
        if self.pos < self.src.len() && matches!(self.src[self.pos], b'e' | b'E') {
            is_float = true;
            self.pos += 1;
            if self.pos < self.src.len() && matches!(self.src[self.pos], b'+' | b'-') {
                self.pos += 1;
            }
            self.scan_digits();
        }
        self.maybe_complex(if is_float { TokenType::Float } else { TokenType::Int })
    }

    fn scan_dot_number(&mut self) -> TokenType {
        self.scan_digits();
        if self.pos < self.src.len() && matches!(self.src[self.pos], b'e' | b'E') {
            self.pos += 1;
            if self.pos < self.src.len() && matches!(self.src[self.pos], b'+' | b'-') {
                self.pos += 1;
            }
            self.scan_digits();
        }
        self.maybe_complex(TokenType::Float)
    }

    #[inline]
    fn maybe_complex(&mut self, base: TokenType) -> TokenType {
        if self.pos < self.src.len() && matches!(self.src[self.pos], b'j' | b'J') {
            self.pos += 1; TokenType::Complex
        } else { base }
    }

    /*
    String Scanning
        Handles single-quote, double-quote and triple-quoted strings with escape awareness.
    */

    fn scan_string(&mut self, quote: u8) {
        if self.at(0) == Some(quote) && self.at(1) == Some(quote) {
            self.pos += 2;
            self.scan_triple_string(quote);
        } else {
            self.scan_single_string(quote);
        }
    }

    fn scan_single_string(&mut self, quote: u8) {
        while self.pos < self.src.len() {
            let b = self.src[self.pos];
            if b == quote { self.pos += 1; return; }
            if b == b'\\' { self.pos += 1; }
            if b == b'\n' { break; }
            self.pos += 1;
        }
    }

    fn scan_triple_string(&mut self, quote: u8) {
        while self.pos < self.src.len() {
            let b = self.src[self.pos];
            if b == quote && self.at(1) == Some(quote) && self.at(2) == Some(quote) {
                self.pos += 3; return;
            }
            if b == b'\\' { self.pos += 1; }
            if b == b'\n' { self.line += 1; }
            self.pos += 1;
        }
    }

    /*
    F-String Scanning
        Emits FstringStart/Middle/End tokens and suspends at `{` for expression lexing.
    */

    fn start_fstring(&mut self, start: usize, prefix_end: usize) {
        let quote = self.src[prefix_end];
        let triple = self.src.get(prefix_end + 1) == Some(&quote) && self.src.get(prefix_end + 2) == Some(&quote);
        let quote_len = if triple { 3 } else { 1 };
        self.pos = prefix_end + quote_len;
        let body_start = self.pos;
        self.scan_fstring_body(quote, triple, body_start);
        self.pending.push((TokenType::FstringStart, self.line, start, body_start));
    }

    fn scan_fstring_body(&mut self, quote: u8, triple: bool, body_start: usize) {
        let mut pos = self.pos;
        while pos < self.src.len() {
            let closes = if triple {
                pos + 2 < self.src.len()
                    && self.src[pos] == quote
                    && self.src[pos + 1] == quote
                    && self.src[pos + 2] == quote
            } else {
                self.src[pos] == quote
            };
            if closes {
                let ql = if triple { 3 } else { 1 };
                self.pending.push((TokenType::FstringEnd, self.line, pos, pos + ql));
                if pos > self.pos {
                    self.pending.push((TokenType::FstringMiddle, self.line, body_start, pos));
                }
                self.pos = pos + ql;
                return;
            }
            match self.src[pos] {
                b'\\' => pos = (pos + 2).min(self.src.len()),
                b'{' if self.src.get(pos + 1) != Some(&b'{') => {
                    if self.fstring_stack.len() >= MAX_FSTRING_DEPTH {
                        self.pending.push((TokenType::Endmarker, self.line, pos, pos));
                        if pos > self.pos {
                            self.pending.push((TokenType::FstringMiddle, self.line, body_start, pos));
                        }
                        self.pos = pos + 1;
                        return;
                    }
                    self.pending.push((TokenType::Lbrace, self.line, pos, pos + 1));
                    if pos > self.pos {
                        self.pending.push((TokenType::FstringMiddle, self.line, body_start, pos));
                    }
                    self.fstring_stack.push((quote, triple, pos + 1, self.nesting));
                    self.pos = pos + 1;
                    return;
                }
                b'\n' => { self.line += 1; pos += 1; }
                _ => pos += 1,
            }
        }
        self.pos = pos;
    }

    /*
    Indentation Handling
        Emits Newline/Indent/Dedent or suppresses them inside bracketed expressions.
    */

    fn handle_newline(&mut self, start: usize) {
        let current_line = self.line;
        self.line += 1;

        if self.nesting > 0 {
            self.pending.push((TokenType::Nl, current_line, start, self.pos));
            return;
        }

        let mut level = 0usize;
        let mut has_space = false;
        let mut has_tab = false;
        let mut p = self.pos;
        while p < self.src.len() && (self.src[p] == b' ' || self.src[p] == b'\t') {
            has_space |= self.src[p] == b' ';
            has_tab   |= self.src[p] == b'\t';
            level += 1; p += 1;
        }

        if has_space && has_tab {
            self.pending.push((TokenType::Endmarker, current_line, start, self.pos)); // sale 2.º
            self.pending.push((TokenType::Newline,   current_line, start, self.pos)); // sale 1.º
            return;
        }

        if matches!(self.src.get(p), Some(b'\n' | b'\r' | b'#')) {
            self.pending.push((TokenType::Nl, current_line, start, self.pos));
            return;
        }

        let line_pos = self.pos + level;
        let current = *self.indent_stack.last().unwrap_or(&0);

        match level.cmp(&current) {
            Ordering::Greater => {
                if self.indent_stack.len() >= MAX_INDENT_DEPTH {
                    self.pending.push((TokenType::Endmarker, current_line, start, self.pos)); // sale 2.º
                    self.pending.push((TokenType::Newline,   current_line, start, self.pos)); // sale 1.º
                    return;
                }
                self.indent_stack.push(level);
                self.pending.push((TokenType::Indent,  self.line,    line_pos, line_pos)); // sale 2.º
                self.pending.push((TokenType::Newline,  current_line, start,    self.pos)); // sale 1.º
            }
            Ordering::Less => {
                while self.indent_stack.last().is_some_and(|&t| t > level) {
                    self.indent_stack.pop();
                    self.pending.push((TokenType::Dedent, self.line, line_pos, line_pos)); // salen últimos
                }
                self.pending.push((TokenType::Newline, current_line, start, self.pos)); // sale 1.º
            }
            Ordering::Equal => {
                self.pending.push((TokenType::Newline, current_line, start, self.pos));
            }
        }
    }

    /*
    Close Brace
        Routes `}` to f-string body resume or plain Rbrace based on nesting depth.
    */

    fn close_brace(&mut self, start: usize) {
        let end = start + 1;
        if let Some(&(_, _, _, saved_nesting)) = self.fstring_stack.last() {
            if self.nesting > saved_nesting {
                self.nesting -= 1;
                self.pending.push((TokenType::Rbrace, self.line, start, end));
            } else {
                let (quote, triple, _, _) = self.fstring_stack.pop().unwrap();
                self.pos = end;
                self.scan_fstring_body(quote, triple, end);
                self.pending.push((TokenType::Rbrace, self.line, start, end));
                return;
            }
        } else {
            self.nesting = self.nesting.saturating_sub(1);
            self.pending.push((TokenType::Rbrace, self.line, start, end));
        }
        self.pos = end;
    }

    /*
    Main Dispatch
        Routes each byte via BYTE_CLASS and SINGLE_TOK, drains pending queue first.
    */

    pub fn next_token(&mut self) -> Option<(TokenType, usize, usize, usize)> {
        if let Some(tok) = self.pending.pop() {
            return Some(tok);
        }

        self.skip_whitespace();
        if self.pos >= self.src.len() { return Option::None; }

        let start = self.pos;
        let b = self.src[self.pos];
        let cl = BYTE_CLASS[b as usize];

        // Newline
        if b == b'\n' {
            self.pos += 1;
            self.handle_newline(start);
            return self.pending.pop();
        }

        // Comment
        if b == b'#' {
            while self.pos < self.src.len() && self.src[self.pos] != b'\n' {
                self.pos += 1;
            }
            return Some((TokenType::Comment, self.line, start, self.pos));
        }

        // Identifier / keyword / string-prefix / f-string-prefix
        if cl & ID_START != 0 {
            self.pos += if b >= 0x80 { utf8_char_len(b) } else { 1 };
            self.scan_id_rest();
            let slice = &self.src[start..self.pos];

            if is_fstring_prefix(slice) {
                if let Some(&q) = self.src.get(self.pos) {
                    if q == b'"' || q == b'\'' {
                        let pe = self.pos;
                        self.start_fstring(start, pe);
                        return self.pending.pop();
                    }
                }
            }

            if is_string_prefix(slice) {
                if let Some(&q) = self.src.get(self.pos) {
                    if q == b'"' || q == b'\'' {
                        self.pos += 1;
                        self.scan_string(q);
                        return Some((TokenType::String, self.line, start, self.pos));
                    }
                }
            }

            let kind = keyword(slice).unwrap_or(TokenType::Name);
            return Some((kind, self.line, start, self.pos));
        }

        // Number (digit start)
        if cl & DIGIT != 0 {
            self.pos += 1;
            let kind = self.scan_number(start);
            return Some((kind, self.line, start, self.pos));
        }

        // Ellipsis
        if b == b'.' && self.at(1) == Some(b'.') && self.at(2) == Some(b'.') {
            self.pos += 3;
            return Some((TokenType::Ellipsis, self.line, start, self.pos));
        }

        // Dot-number (.123)
        if b == b'.' && self.at(1).map_or(false, |c| BYTE_CLASS[c as usize] & DIGIT != 0) {
            self.pos += 1;
            let kind = self.scan_dot_number();
            return Some((kind, self.line, start, self.pos));
        }

        // Bare string
        if b == b'"' || b == b'\'' {
            self.pos += 1;
            self.scan_string(b);
            return Some((TokenType::String, self.line, start, self.pos));
        }

        // Close brace (f-string aware)
        if b == b'}' {
            self.close_brace(start);
            return self.pending.pop();
        }

        // Multi-char operators: 3-char
        if self.pos + 2 < self.src.len() {
            let kind = match &self.src[self.pos..self.pos + 3] {
                b"**=" => Some(TokenType::DoubleStarEqual),
                b"//=" => Some(TokenType::DoubleSlashEqual),
                b"<<=" => Some(TokenType::LeftShiftEqual),
                b">>=" => Some(TokenType::RightShiftEqual),
                _ => Option::None,
            };
            if let Some(k) = kind {
                self.pos += 3;
                return Some((k, self.line, start, self.pos));
            }
        }

        // Multi-char operators: 2-char
        if self.pos + 1 < self.src.len() {
            let kind = match &self.src[self.pos..self.pos + 2] {
                b"!=" => Some(TokenType::NotEqual), b"%=" => Some(TokenType::PercentEqual),
                b"&=" => Some(TokenType::AmperEqual), b"**" => Some(TokenType::DoubleStar),
                b"*=" => Some(TokenType::StarEqual), b"+=" => Some(TokenType::PlusEqual),
                b"-=" => Some(TokenType::MinEqual), b"->" => Some(TokenType::Rarrow),
                b"//" => Some(TokenType::DoubleSlash), b"/=" => Some(TokenType::SlashEqual),
                b":=" => Some(TokenType::ColonEqual), b"<<" => Some(TokenType::LeftShift),
                b"<=" => Some(TokenType::LessEqual), b"==" => Some(TokenType::EqEqual),
                b">=" => Some(TokenType::GreaterEqual),b">>" => Some(TokenType::RightShift),
                b"@=" => Some(TokenType::AtEqual), b"^=" => Some(TokenType::CircumflexEqual),
                b"|=" => Some(TokenType::VbarEqual),
                _ => Option::None,
            };
            if let Some(k) = kind {
                self.pos += 2;
                return Some((k, self.line, start, self.pos));
            }
        }

        // Single-char: table dispatch
        self.pos += 1;
        let idx = if b < 128 { SINGLE_TOK[b as usize] } else { 0 };
        let kind = SINGLE_MAP[idx as usize];

        match kind {
            TokenType::Lpar | TokenType::Lsqb | TokenType::Lbrace => self.nesting += 1,
            TokenType::Rpar | TokenType::Rsqb => self.nesting = self.nesting.saturating_sub(1),
            _ => {}
        }

        Some((kind, self.line, start, self.pos))
    }
}