// parser/expr.rs

use super::Parser;
use super::types::{OpCode, Value, MAX_EXPR_DEPTH, Instruction};
use super::types::parse_string;
use crate::modules::lexer::{Token, TokenType};
use alloc::{string::ToString, vec::Vec, vec, format, string::String};

impl<'src, I: Iterator<Item = Token>> Parser<'src, I> {

    /*
    Expression Entry Point
        Main driver for expressions with recursion guard, ternary support and tails.
    */

    pub(super) fn expr(&mut self) {
        self.expr_depth += 1;
        // A04:2021 - Insecure Design: cap recursion depth to prevent stack overflow.
        if self.expr_depth > MAX_EXPR_DEPTH {
            self.expr_depth -= 1;
            self.error("expression too deeply nested");
            return;
        }
        self.saw_newline = false;
        self.expr_bp(0);
        if !self.saw_newline && matches!(self.peek(), Some(TokenType::If)) {
            self.advance();
            self.expr_bp(0);
            self.chunk.emit(OpCode::JumpIfFalse, 0);
            let jf = self.chunk.instructions.len() - 1;
            self.chunk.emit(OpCode::Jump, 0);
            let jmp = self.chunk.instructions.len() - 1;
            self.patch(jf);
            self.chunk.emit(OpCode::PopTop, 0);
            self.eat(TokenType::Else);
            self.expr_bp(0);
            self.patch(jmp);
        }
        self.expr_depth -= 1;
    }

    pub(super) fn expr_tails(&mut self) {
        self.postfix_tail();
        self.infix_bp(0);
    }

    /*
    Pratt Parser
        Implements Pratt parsing for infix operators using binding powers and precedence.
    */

    pub(super) fn expr_bp(&mut self, min_bp: u8) {
        match self.peek() {
            Some(TokenType::Not) => {
                self.advance();
                self.expr_bp(5);
                self.chunk.emit(OpCode::Not, 0);
            }
            _ => self.parse_unary(),
        }
        self.infix_bp(min_bp);
    }

    pub(super) fn infix_bp(&mut self, min_bp: u8) {
        while let Some(tok) = self.peek() {
            // `is` / `is not`
            if tok == TokenType::Is {
                if 7 < min_bp { break; }
                self.advance();
                if self.eat_if(TokenType::Not) {
                    self.expr_bp(8);
                    self.chunk.emit(OpCode::IsNot, 0);
                } else {
                    self.expr_bp(8);
                    self.chunk.emit(OpCode::Is, 0);
                }
                continue;
            }

            // `not in`
            if tok == TokenType::Not {
                if 7 < min_bp { break; }
                self.advance();
                self.eat(TokenType::In);
                self.expr_bp(8);
                self.chunk.emit(OpCode::NotIn, 0);
                continue;
            }

            let Some((l_bp, r_bp, op)) = Self::binding_power(&tok) else { break };
            if l_bp < min_bp { break; }
            self.advance();

            // Short-circuit: peek-and-jump preserves the actual value (Python semantics).
            if matches!(op, OpCode::And | OpCode::Or) {
                let jump_op = if op == OpCode::And { OpCode::JumpIfFalseOrPop } else { OpCode::JumpIfTrueOrPop };
                self.chunk.emit(jump_op, 0);
                let jmp = self.chunk.instructions.len() - 1;
                self.expr_bp(r_bp);
                self.patch(jmp);
                continue;
            }

            self.expr_bp(r_bp);

            // Chained comparisons: `a < b < c` => `(a < b) and (b < c)` with b evaluated once.
            // Store b in a temp SSA slot so it can be reused as LHS of the next comparison.
            if matches!(op, OpCode::Eq | OpCode::NotEq | OpCode::Lt | OpCode::Gt
                | OpCode::LtEq | OpCode::GtEq)
            {
                if let Some(next_tok) = self.peek() {
                    if matches!(next_tok, TokenType::Less | TokenType::Greater
                        | TokenType::LessEqual | TokenType::GreaterEqual
                        | TokenType::EqEqual | TokenType::NotEqual)
                    {
                        let ver = self.increment_version("__cmp__");
                        let tmp = self.push_ssa_name("__cmp__", ver);
                        self.chunk.emit(OpCode::StoreName, tmp);  // save b
                        self.chunk.emit(OpCode::LoadName, tmp);   // reload b for this comparison
                        self.chunk.emit(op, 0);                   // emit a < b
                        self.chunk.emit(OpCode::JumpIfFalseOrPop, 0);
                        let jmp = self.chunk.instructions.len() - 1;
                        self.chunk.emit(OpCode::LoadName, tmp);   // b becomes LHS for next
                        self.infix_bp(min_bp);
                        self.patch(jmp);
                        return;
                    }
                }
            }

            self.chunk.emit(op, 0);
        }
    }

    pub(super) fn binding_power(tok: &TokenType) -> Option<(u8, u8, OpCode)> {
        match tok {
            TokenType::Or => Some((1, 2, OpCode::Or)),
            TokenType::And => Some((3, 4, OpCode::And)),
            TokenType::EqEqual => Some((7, 8, OpCode::Eq)),
            TokenType::NotEqual => Some((7, 8, OpCode::NotEq)),
            TokenType::Less => Some((7, 8, OpCode::Lt)),
            TokenType::Greater => Some((7, 8, OpCode::Gt)),
            TokenType::LessEqual => Some((7, 8, OpCode::LtEq)),
            TokenType::GreaterEqual => Some((7, 8, OpCode::GtEq)),
            TokenType::In => Some((7, 8, OpCode::In)),
            TokenType::Vbar => Some((9, 10, OpCode::BitOr)),
            TokenType::Circumflex => Some((11, 12, OpCode::BitXor)),
            TokenType::Amper => Some((13, 14, OpCode::BitAnd)),
            TokenType::LeftShift => Some((15, 16, OpCode::Shl)),
            TokenType::RightShift => Some((15, 16, OpCode::Shr)),
            TokenType::Plus => Some((17, 18, OpCode::Add)),
            TokenType::Minus => Some((17, 18, OpCode::Sub)),
            TokenType::Star => Some((19, 20, OpCode::Mul)),
            TokenType::Slash => Some((19, 20, OpCode::Div)),
            TokenType::Percent => Some((19, 20, OpCode::Mod)),
            TokenType::DoubleSlash => Some((19, 20, OpCode::FloorDiv)),
            TokenType::DoubleStar => Some((22, 21, OpCode::Pow)),
            _ => None
        }
    }

    /*
    Unary Parser
        Recursively handles unary minus, bitwise not and await operators.
    */

    pub(super) fn parse_unary(&mut self) {
        match self.peek() {
            Some(TokenType::Minus) => {
                self.advance();
                // Python: `**` binds tighter than unary `-` on its left.
                // `-2**3` parses as `-(2**3)`, not `(-2)**3`.
                // We parse the operand with min_bp = pow_right_bp (21) so `**`
                // (left_bp=22) gets to consume the atom first.
                self.expr_bp(21);
                self.chunk.emit(OpCode::Minus, 0);
            }
            Some(TokenType::Tilde) => {
                self.advance();
                self.expr_bp(21);
                self.chunk.emit(OpCode::BitNot, 0);
            }
            Some(TokenType::Await) => {
                self.advance();
                self.expr_bp(21);
                self.chunk.emit(OpCode::Await, 0);
            }
            _ => self.parse_atom(),
        }
    }

    /*
    Atom Parser
        Parses primary atoms: literals, names, numbers, strings, f-strings and containers.
    */

    pub(super) fn parse_atom(&mut self) {
        let t = self.advance();
        match t.kind {
            TokenType::Name => self.name(t),
            TokenType::String => {
                let mut s = parse_string(self.lexeme(&t));
                while matches!(self.peek(), Some(TokenType::String)) {
                    let t = self.advance();
                    s.push_str(&parse_string(self.lexeme(&t)));
                }
                self.emit_const(Value::Str(s));
            }
            TokenType::Complex => {
                let raw = self.lexeme(&t).replace('_', "");
                let s = raw.trim_end_matches(['j', 'J']);
                self.emit_const(Value::Float(s.parse().unwrap_or(0.0)));
            }
            TokenType::Int | TokenType::Float => {
                self.parse_number(self.lexeme(&t), t.kind);
            }
            TokenType::True => self.chunk.emit(OpCode::LoadTrue, 0),
            TokenType::False => self.chunk.emit(OpCode::LoadFalse, 0),
            TokenType::None => self.chunk.emit(OpCode::LoadNone, 0),
            TokenType::Ellipsis => self.chunk.emit(OpCode::LoadEllipsis, 0),
            TokenType::FstringStart => self.fstring(),
            TokenType::Lbrace => self.brace_literal(),
            TokenType::Lsqb => self.list_literal(),
            TokenType::Lpar => {
                if matches!(self.peek(), Some(TokenType::Rpar)) {
                    self.advance();
                    self.chunk.emit(OpCode::BuildTuple, 0);
                } else {
                    let elem_start = self.chunk.instructions.len();
                    self.expr();
                    if matches!(self.peek(), Some(TokenType::For)) {
                        let versions_before = self.ssa_versions.clone();
                        let elem_ins: Vec<Instruction> = self.chunk.instructions.drain(elem_start..).collect();
                        self.chunk.emit(OpCode::BuildList, 0);
                        self.comprehension_loop(&[elem_ins], OpCode::ListAppend, &versions_before);
                        self.advance(); // Rpar
                    } else if self.eat_if(TokenType::Comma) {
                        let mut count = 1u16;
                        while !matches!(self.peek(), Some(TokenType::Rpar) | None) {
                            self.expr();
                            count += 1;
                            if !self.eat_if(TokenType::Comma) { break; }
                        }
                        self.eat(TokenType::Rpar);
                        self.chunk.emit(OpCode::BuildTuple, count);
                    } else {
                        self.eat(TokenType::Rpar);
                    }
                }
            }
            TokenType::Lambda => self.parse_lambda(),
            _ => {
                if t.kind != TokenType::Endmarker {
                    self.error("unexpected token");
                }
            }
        }
        self.postfix_tail();
    }

    /*
    Name Handler
        Special parsing for names including assignment, walrus operator and function calls.
    */

    pub(super) fn name(&mut self, t: Token) {
        let name = self.lexeme(&t).to_string();
        match self.peek() {
            Some(TokenType::Equal) => {
                self.assign(name.clone());
                self.emit_load_ssa(name);
            }
            Some(TokenType::ColonEqual) => {
                self.advance();
                self.expr();
                let ver = self.increment_version(&name);
                let mut buf = [0u8; 128];
                let i = self.chunk.push_name(Self::ssa_name(&name, ver, &mut buf));
                self.chunk.emit(OpCode::StoreName, i);
                self.chunk.emit(OpCode::LoadName, i);
            }
            Some(TokenType::Lpar) => {
                let _ = self.call(name);
            }
            _ => self.emit_load_ssa(name),
        }
        self.postfix_tail();
    }

    fn parse_int_prefix(s: &str) -> (&str, u32) {
        if let Some(h) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) { (h, 16) }
        else if let Some(o) = s.strip_prefix("0o").or_else(|| s.strip_prefix("0O")) { (o, 8) }
        else if let Some(b) = s.strip_prefix("0b").or_else(|| s.strip_prefix("0B")) { (b, 2) }
        else { (s, 10) }
    }

    pub(super) fn parse_number(&mut self, raw: &str, kind: TokenType) {
        let s = raw.replace('_', "");
        if kind == TokenType::Float {
            self.emit_const(Value::Float(s.parse().unwrap_or(0.0)));
            return;
        }
        let (digits, base) = Self::parse_int_prefix(&s);
        let maybe = if base == 10 {
            digits.parse().ok()
        } else {
            i64::from_str_radix(digits, base).ok()
        };
        match maybe {
            Some(v) => self.emit_const(Value::Int(v)),
            None => {
                let dec = if base == 10 { digits.to_string() } else { Self::big_base_to_dec(digits, base) };
                self.emit_const(Value::BigInt(dec));
            }
        }
    }

    fn big_base_to_dec(s: &str, base: u32) -> String {
        const DEC: u64 = 1_000_000_000;
        let mut limbs: Vec<u32> = vec![0];
        for c in s.chars() {
            let d = c.to_digit(base).unwrap_or(0) as u64;
            let mut carry = d;
            for limb in limbs.iter_mut() {
                let cur = *limb as u64 * base as u64 + carry;
                *limb = (cur % DEC) as u32;
                carry = cur / DEC;
            }
            if carry != 0 { limbs.push(carry as u32); }
        }
        let mut out = String::new();
        for (i, &l) in limbs.iter().rev().enumerate() {
            if i == 0 { out.push_str(&format!("{}", l)); }
            else { out.push_str(&format!("{:09}", l)); }
        }
        if out.is_empty() { out.push('0'); }
        out
    }

    /*
    Postfix Tail
        Handles attribute access, indexing, slicing and method calls after atoms.
    */
    
    pub(super) fn postfix_tail(&mut self) {
        loop {
            match self.peek() {
                Some(TokenType::Lsqb) => {
                    self.advance();
                    let is_slice = matches!(self.peek(), Some(TokenType::Colon));
                    if is_slice {
                        self.chunk.emit(OpCode::LoadNone, 0);
                    } else {
                        self.expr();
                    }
                    if self.eat_if(TokenType::Colon) {
                        let mut parts = 2u16;
                        if matches!(self.peek(), Some(TokenType::Colon | TokenType::Rsqb)) {
                            self.chunk.emit(OpCode::LoadNone, 0);
                        } else {
                            self.expr();
                        }
                        if self.eat_if(TokenType::Colon) {
                            parts = 3;
                            if matches!(self.peek(), Some(TokenType::Rsqb)) {
                                self.chunk.emit(OpCode::LoadNone, 0);
                            } else {
                                self.expr();
                            }
                        }
                        self.eat(TokenType::Rsqb);
                        self.chunk.emit(OpCode::BuildSlice, parts);
                        self.chunk.emit(OpCode::GetItem, 0);
                    } else {
                        self.eat(TokenType::Rsqb);
                        self.chunk.emit(OpCode::GetItem, 0);
                    }
                }
                Some(TokenType::Dot) => {
                    self.advance();
                    let t = self.advance();
                    let (start, end) = (t.start, t.end);
                    let idx = self.chunk.push_name(&self.source[start..end]);
                    self.chunk.emit(OpCode::LoadAttr, idx);
                    if matches!(self.peek(), Some(TokenType::Lpar)) {
                        let (pos, kw) = self.parse_args();
                        let encoded = ((kw & 0xFF) << 8) | (pos & 0xFF);
                        self.chunk.emit(OpCode::Call, encoded);
                    }
                }
                _ => break,
            }
        }
    }

    /*
    Lambda Parser
        Parses lambda expressions by compiling body into a MakeFunction object.
    */

    pub(super) fn parse_lambda(&mut self) {
        let mut params = Vec::new();
        if !matches!(self.peek(), Some(TokenType::Colon)) {
            loop {
                let p = self.advance();
                params.push(self.lexeme(&p).to_string());
                if !self.eat_if(TokenType::Comma) {
                    break;
                }
            }
        }
        self.eat(TokenType::Colon);

        let body = self.with_fresh_chunk(|s| {
            for p in &params { s.ssa_versions.insert(p.clone(), 0); }
            s.expr();
            s.chunk.emit(OpCode::ReturnValue, 0);
        });
        let fi = self.chunk.functions.len() as u16;
        self.chunk.functions.push((params, body, 0, u16::MAX));
        self.chunk.emit(OpCode::MakeFunction, fi);
    }
}