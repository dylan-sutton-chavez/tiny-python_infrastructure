// parser/literals.rs

use super::Parser;
use super::types::builtin;

use super::types::{OpCode, Value, SSAChunk, Instruction};
use crate::modules::lexer::{Token, TokenType};
use alloc::{string::{String, ToString}, vec::Vec, format};
use hashbrown::HashMap;

impl<'src, I: Iterator<Item = Token>> Parser<'src, I> {

    /*
    Brace Literal Handler
        Parses {} for dict/set literals and dict/set comprehensions.
    */

    pub(super) fn brace_literal(&mut self) {
        if matches!(self.peek(), Some(TokenType::Rbrace)) {
            self.advance();
            self.chunk.emit(OpCode::BuildDict, 0);
            return;
        }
        let key_start = self.chunk.instructions.len();
        self.expr();
        match self.peek() {
            Some(TokenType::Colon) => {
                self.advance();
                let val_start = self.chunk.instructions.len();
                self.expr();
                if matches!(self.peek(), Some(TokenType::For)) {
                    let versions_before = self.ssa_versions.clone();
                    let val_ins: Vec<Instruction> = self.chunk.instructions.drain(val_start..).collect();
                    let key_ins: Vec<Instruction> = self.chunk.instructions.drain(key_start..).collect();
                    self.chunk.emit(OpCode::BuildDict, 0);
                    self.comprehension_loop(&[key_ins, val_ins], OpCode::MapAdd, &versions_before);
                    self.advance(); // Rbrace
                } else {
                    let mut pairs = 1u16;
                    while self.eat_if(TokenType::Comma) {
                        if matches!(self.peek(), Some(TokenType::Rbrace)) { break; }
                        self.expr();
                        self.eat(TokenType::Colon);
                        self.expr();
                        pairs += 1;
                    }
                    self.advance();
                    self.chunk.emit(OpCode::BuildDict, pairs);
                }
            }
            Some(TokenType::For) => {
                let versions_before = self.ssa_versions.clone();
                let elem_ins: Vec<Instruction> = self.chunk.instructions.drain(key_start..).collect();
                self.chunk.emit(OpCode::BuildSet, 0);
                self.comprehension_loop(&[elem_ins], OpCode::SetAdd, &versions_before);
                self.advance(); // Rbrace
            }
            _ => {
                let mut count = 1u16;
                while self.eat_if(TokenType::Comma) {
                    if matches!(self.peek(), Some(TokenType::Rbrace)) { break; }
                    self.expr();
                    count += 1;
                }
                self.advance();
                self.chunk.emit(OpCode::BuildSet, count);
            }
        }
    }

    /*
    List Literal Handler
        Parses [] for list literals and list comprehensions.
    */

    pub(super) fn list_literal(&mut self) {
        if matches!(self.peek(), Some(TokenType::Rsqb)) {
            self.advance();
            self.chunk.emit(OpCode::BuildList, 0);
            return;
        }
        let elem_start = self.chunk.instructions.len();
        self.expr();
        if matches!(self.peek(), Some(TokenType::For)) {
            let versions_before = self.ssa_versions.clone();
            let elem_ins: Vec<Instruction> = self.chunk.instructions.drain(elem_start..).collect();
            self.chunk.emit(OpCode::BuildList, 0);
            self.comprehension_loop(&[elem_ins], OpCode::ListAppend, &versions_before);
            self.advance(); // Rsqb
        } else {
            let mut count = 1u16;
            while self.eat_if(TokenType::Comma) {
                if matches!(self.peek(), Some(TokenType::Rsqb)) { break; }
                self.expr();
                count += 1;
            }
            self.advance();
            self.chunk.emit(OpCode::BuildList, count);
        }
    }

    /*
    Comprehension Loop
        Builds for/if scaffolding and reinjects element body with SSA slot rewriting.
    */

    pub(super) fn comprehension_loop(&mut self, elem_bodies: &[Vec<Instruction>], append_op: OpCode, versions_before: &HashMap<String, u32>) {
        let mut loop_starts: Vec<u16> = Vec::new();
        let mut for_iters: Vec<usize> = Vec::new();
        let mut all_vars: Vec<String> = Vec::new();

        while self.eat_if(TokenType::For) {
            let mut vars: Vec<String> = Vec::new();
            loop {
                let t = self.advance();
                vars.push(self.lexeme(&t).to_string());
                if !self.eat_if(TokenType::Comma) { break; }
                if matches!(self.peek(), Some(TokenType::In)) { break; }
            }

            self.eat(TokenType::In);
            self.expr_bp(1);
            self.chunk.emit(OpCode::GetIter, 0);

            let ls = self.chunk.instructions.len() as u16;
            self.chunk.emit(OpCode::ForIter, 0);
            let fi = self.chunk.instructions.len() - 1;

            if vars.len() == 1 {
                self.store_name(vars[0].clone());
            } else {
                self.chunk.emit(OpCode::UnpackSequence, vars.len() as u16);
                for var in &vars {
                    self.store_name(var.clone());
                }
            }
            for v in &vars { all_vars.push(v.clone()); }

            while self.eat_if(TokenType::If) {
                self.expr_bp(1);
                self.chunk.emit(OpCode::JumpIfFalse, ls); // fail -> next iteration
            }

            loop_starts.push(ls);
            for_iters.push(fi);
        }

        // Map pre-loop slots to current versions, skipping non-existent element references.
        let mut var_map: HashMap<u16, u16> = HashMap::new();
        for var in &all_vars {
            let old_ver = versions_before.get(var).copied().unwrap_or(0);
            let new_ver = self.current_version(var);
            if old_ver == new_ver { continue; }
            let old_name = format!("{}_{}", var, old_ver);
            let Some(&old_slot) = self.chunk.name_index.get(old_name.as_str()) else { continue };
            let mut nb = [0u8; 128];
            let new_slot = self.chunk.push_name(Self::ssa_name(var, new_ver, &mut nb));
            var_map.insert(old_slot, new_slot);
        }

        for body in elem_bodies {
            for ins in body {
                let operand = if matches!(ins.opcode, OpCode::LoadName | OpCode::StoreName) {
                    var_map.get(&ins.operand).copied().unwrap_or(ins.operand)
                } else {
                    ins.operand
                };
                self.chunk.instructions.push(Instruction { opcode: ins.opcode, operand });
            }
        }
        self.chunk.emit(append_op, 0);

        // Close loops innermost-first: Jump back to header, patch matching ForIter to the point past it.
        for i in (0..for_iters.len()).rev() {
            self.chunk.emit(OpCode::Jump, loop_starts[i]);
            self.patch(for_iters[i]);
        }
    }

    /*
    F-String Parser
        Parses f-strings with embedded expressions and format specs.
    */

    pub(super) fn fstring(&mut self) {
        let mut parts = 0u16;
        if matches!(self.peek(), Some(TokenType::FstringEnd)) {
            self.advance();
            self.emit_const(Value::Str(String::new()));
            return;
        }
        loop {
            match self.peek() {
                Some(TokenType::FstringMiddle) => {
                    let t = self.advance();
                    self.emit_const(Value::Str(self.lexeme(&t).to_string()));
                    parts += 1;
                }
                Some(TokenType::Lbrace) => {
                    self.advance();
                    self.expr();
                    if matches!(self.peek(), Some(TokenType::Colon)) {
                        let colon = self.advance();
                        let spec_start = colon.end;
                        loop {
                            match self.tokens.peek().map(|t| t.kind) {
                                Some(TokenType::Rbrace) | None => break,
                                _ => { self.tokens.next(); }
                            }
                        }
                        let spec_end = self.tokens.peek().map(|t| t.start).unwrap_or(spec_start);
                        let spec = self.source[spec_start..spec_end].to_string();
                        let idx = self.chunk.push_const(Value::Str(spec));
                        self.chunk.emit(OpCode::LoadConst, idx);
                        self.chunk.emit(OpCode::FormatValue, 1);
                    } else {
                        self.chunk.emit(OpCode::FormatValue, 0);
                    }
                    parts += 1;
                    if matches!(self.peek(), Some(TokenType::Rbrace)) {
                        self.advance();
                    }
                }
                Some(TokenType::FstringEnd) => {
                    self.advance();
                    break;
                }
                _ => break
            }
        }
        if parts > 0 {
            self.chunk.emit(OpCode::BuildString, parts);
        }
    }

    /*
    Function Call Handler
        Dispatches print/range/builtins or general function calls with args.
    */

    pub(super) fn call(&mut self, name: String) -> bool {
        if name == "print" {
            let (argc, _) = self.parse_args();
            self.chunk.emit(OpCode::CallPrint, argc);
            return false;
        }

        if name == "range" {
            self.call_range();
            return true;
        }

        if let Some((op, leaves_value)) = builtin(name.as_str()) {
            let (a, _) = self.parse_args();
            self.chunk.emit(op, a);
            return leaves_value;
        }

        let v = self.current_version(&name);
        let mut buf = [0u8; 128];
        let i = self.chunk.push_name(Self::ssa_name(&name, v, &mut buf));
        self.chunk.emit(OpCode::LoadName, i);
        let (a, kw) = self.parse_args();
        self.chunk.emit(OpCode::Call, a + kw);
        true
    }

    pub(super) fn call_range(&mut self) {
        self.advance();
        let mut argc = 0u16;
        while !matches!(self.peek(), Some(TokenType::Rpar) | None) {
            self.expr();
            argc += 1;
            if matches!(self.peek(), Some(TokenType::Comma)) {
                self.advance();
            }
        }
        self.advance();
        self.chunk.emit(OpCode::CallRange, argc);
    }

    pub(super) fn parse_args(&mut self) -> (u16, u16) {
        self.advance();
        let mut argc = 0;
        let mut kwargs = 0u16;
        while !matches!(self.peek(), Some(TokenType::Rpar) | None) {
            if self.eat_if(TokenType::Star) {
                self.expr();
                self.chunk.emit(OpCode::UnpackArgs, 1);
            } else if self.eat_if(TokenType::DoubleStar) {
                self.expr();
                self.chunk.emit(OpCode::UnpackArgs, 2);
            } else if matches!(self.peek(), Some(TokenType::Name)) {
                let t = self.advance();
                if matches!(self.peek(), Some(TokenType::Equal)) {
                    self.advance();
                    let i = self.chunk.push_const(Value::Str(self.lexeme(&t).to_string()));
                    self.chunk.emit(OpCode::LoadConst, i);
                    self.expr();
                    kwargs += 1;
                } else {
                    // Support genexpr after name arg (expr for ...)
                    let elem_start = self.chunk.instructions.len();
                    self.name(t);
                    self.infix_bp(0);
                    if matches!(self.peek(), Some(TokenType::For)) {
                        let versions_before = self.ssa_versions.clone();
                        let elem_ins: Vec<Instruction> = self.chunk.instructions.drain(elem_start..).collect();
                        self.chunk.emit(OpCode::BuildList, 0);
                        self.comprehension_loop(&[elem_ins], OpCode::ListAppend, &versions_before);
                    }
                }
            } else {
                // Support genexpr in arg position (expr for ...)
                let elem_start = self.chunk.instructions.len();
                self.expr();
                if matches!(self.peek(), Some(TokenType::For)) {
                    let versions_before = self.ssa_versions.clone();
                    let elem_ins: Vec<Instruction> = self.chunk.instructions.drain(elem_start..).collect();
                    self.chunk.emit(OpCode::BuildList, 0);
                    self.comprehension_loop(&[elem_ins], OpCode::ListAppend, &versions_before);
                }
            }
            argc += 1;
            if matches!(self.peek(), Some(TokenType::Comma)) {
                self.advance();
            }
        }
        self.eat(TokenType::Rpar);
        (argc, kwargs)
    }

    /*
    Class Definition
        Parses class header, compiles body separately and emits MakeClass.
    */

    pub(super) fn class_def(&mut self) {
        let cname = {
            let n = self.advance();
            self.lexeme(&n).to_string()
        };

        if self.eat_if(TokenType::Lpar) {
            while !matches!(self.peek(), Some(TokenType::Rpar) | None) {
                self.expr();
                if !self.eat_if(TokenType::Comma) {
                    break;
                }
            }
            self.eat(TokenType::Rpar);
        }

        self.eat(TokenType::Colon);

        let saved_chunk = core::mem::take(&mut self.chunk);
        let saved_ver = core::mem::take(&mut self.ssa_versions);
        self.ssa_versions = saved_ver.clone();

        self.compile_block();

        let body = core::mem::take(&mut self.chunk);
        self.chunk = saved_chunk;
        self.ssa_versions = saved_ver;

        let ci = self.chunk.classes.len() as u16;
        self.chunk.classes.push(body);
        self.chunk.emit(OpCode::MakeClass, ci);

        let ver = self.increment_version(&cname);
        let mut buf = [0u8; 128];
        let i = self.chunk.push_name(Self::ssa_name(&cname, ver, &mut buf));
        self.chunk.emit(OpCode::StoreName, i);
    }

    /*
    Function Definition
        Parses params/defaults, compiles body and emits MakeFunction or coroutine.
    */

    pub(super) fn func_def_inner(&mut self, decorators: u16, is_async: bool) {
        let fname = {
            let n = self.advance();
            self.lexeme(&n).to_string()
        };
        let (params, defaults) = self.parse_params();
        let body = self.compile_body(&params);

        let fi = self.chunk.functions.len() as u16;
        let cur_ver = self.current_version(&fname);
        let mut buf = [0u8; 128];
        let name_slot = self.chunk.push_name(Self::ssa_name(&fname, cur_ver + 1, &mut buf));
        self.chunk.functions.push((params, body, defaults, name_slot));
        self.chunk.emit(if is_async { OpCode::MakeCoroutine } else { OpCode::MakeFunction }, fi);

        for _ in 0..decorators {
            self.chunk.emit(OpCode::Call, 1);
        }

        let ver = self.increment_version(&fname);
        let mut buf = [0u8; 128];
        let i = self.chunk.push_name(Self::ssa_name(&fname, ver, &mut buf));
        self.chunk.emit(OpCode::StoreName, i);
    }

    pub(super) fn parse_params(&mut self) -> (Vec<String>, u16) {
        self.advance();
        let mut params = Vec::new();
        let mut defaults = 0u16;
        while !matches!(self.peek(), Some(TokenType::Rpar) | None) {
            if self.eat_if(TokenType::Slash) {
                if matches!(self.peek(), Some(TokenType::Comma)) {
                    self.advance();
                }
                continue;
            }
            if self.eat_if(TokenType::Star) {
                let p = self.advance();
                params.push(format!("*{}", self.lexeme(&p)));
                self.drain_annotation();
            } else if self.eat_if(TokenType::DoubleStar) {
                let p = self.advance();
                params.push(format!("**{}", self.lexeme(&p)));
                self.drain_annotation();
            } else {
                let p = self.advance();
                params.push(self.lexeme(&p).to_string());
                self.drain_annotation();
                if self.eat_if(TokenType::Equal) {
                    self.expr();
                    defaults += 1;
                }
            }
            if matches!(self.peek(), Some(TokenType::Comma)) {
                self.advance();
            }
        }
        self.advance();
        if self.eat_if(TokenType::Rarrow) {
            while !matches!(self.peek(), Some(TokenType::Colon) | None) {
                self.advance();
            }
        }
        if matches!(self.peek(), Some(TokenType::Colon)) {
            self.advance();
        }
        (params, defaults)
    }

    pub(super) fn drain_annotation(&mut self) {
        if self.eat_if(TokenType::Colon) {
            while !matches!(
                self.peek(),
                Some(TokenType::Equal | TokenType::Comma | TokenType::Rpar) | None
            ) {
                self.advance();
            }
        }
    }

    pub(super) fn compile_body(&mut self, params: &[String]) -> SSAChunk {
        let saved_chunk = core::mem::take(&mut self.chunk);
        let saved_ver = core::mem::take(&mut self.ssa_versions);

        self.ssa_versions = saved_ver.clone();
        for p in params {
            self.ssa_versions.insert(p.clone(), 0);
        }

        self.compile_block();

        let mut body = core::mem::take(&mut self.chunk);
        self.chunk = saved_chunk;
        self.ssa_versions = saved_ver;

        body.is_pure = !body.instructions.iter().any(|i| matches!(
            i.opcode,
            OpCode::CallPrint | OpCode::StoreItem | OpCode::StoreAttr | OpCode::CallInput
        ));

        body
    }
}