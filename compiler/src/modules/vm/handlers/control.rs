// vm/handlers/control.rs

use super::*;

pub(crate) enum ControlOutcome {
    Continue,
    Jump(usize),
    Return(Val),
}

impl<'a> VM<'a> {
    pub(crate) fn handle_control(&mut self, op: OpCode, operand: u16, n: usize) -> Result<ControlOutcome, VmErr> {
        match op {
            OpCode::JumpIfFalse => {
                let v = self.pop()?;
                if self.truthy(v) {
                    Ok(ControlOutcome::Continue)
                } else {
                    Ok(ControlOutcome::Jump(self.checked_jump(operand as usize, n)?))
                }
            }
            OpCode::Jump => Ok(ControlOutcome::Jump(self.checked_jump(operand as usize, n)?)),
            OpCode::PopTop => {
                self.pop()?;
                Ok(ControlOutcome::Continue)
            }
            OpCode::Dup2 => {
                let b = self.pop()?;
                let a = self.pop()?;
                self.push(a); self.push(b);
                self.push(a); self.push(b);
                Ok(ControlOutcome::Continue)
            }
            OpCode::ReturnValue => {
                let result = if self.stack.is_empty() { Val::none() } else { self.pop()? };
                Ok(ControlOutcome::Return(result))
            }
            _ => unreachable!("non-control opcode in handle_control"),
        }
    }

    pub(crate) fn handle_iter(&mut self, op: OpCode, operand: u16, n: usize, slots: &mut [Option<Val>]) -> Result<ControlOutcome, VmErr> {
        match op {
            OpCode::GetIter => {
                let obj = self.pop()?;
                let frame = self.make_iter_frame(obj)?;
                self.iter_stack.push(frame);
                Ok(ControlOutcome::Continue)
            }
            OpCode::ForIter => {
                if self.budget == 0 { return Err(cold_budget()); }
                self.budget -= 1;
                if self.heap.needs_gc() { self.collect(slots); }
                match self.iter_stack.last_mut().and_then(|f| f.next_item()) {
                    Some(item) => {
                        self.push(item);
                        Ok(ControlOutcome::Continue)
                    }
                    None => {
                        self.iter_stack.pop();
                        if operand as usize > n {
                            return Err(VmErr::Runtime("jump target out of bounds"));
                        }
                        Ok(ControlOutcome::Jump(operand as usize))
                    }
                }
            }
            _ => unreachable!("non-iter opcode in handle_iter"),
        }
    }
}