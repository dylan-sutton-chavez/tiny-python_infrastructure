// vm/handlers/data.rs

use super::*;

impl<'a> VM<'a> {

    /*
    Store
        StoreName con back-propagación SSA a versiones previas.
    */
    
    pub(crate) fn handle_store(&mut self, operand: u16, slots: &mut [Option<Val>], prev_slots: &[Option<u16>]) -> Result<(), VmErr> {
        let v = self.pop()?;
        let slot = operand as usize;
        slots[slot] = Some(v);

        let mut cur = slot;
        let mut guard = prev_slots.len();
        while guard > 0 {
            guard -= 1;
            match prev_slots.get(cur).and_then(|p| *p) {
                Some(prev) if (prev as usize) != cur => {
                    slots[prev as usize] = Some(v);
                    cur = prev as usize;
                }
                _ => break,
            }
        }
        Ok(())
    }

    /*
    Build
        Constructores de containers: list/tuple/dict/set/slice/string.
    */

    pub(crate) fn handle_build(&mut self, op: OpCode, operand: u16) -> Result<(), VmErr> {
        match op {
            OpCode::BuildList => {
                let v = self.pop_n(operand as usize)?;
                let val = self.heap.alloc(HeapObj::List(Rc::new(RefCell::new(v))))?;
                self.push(val);
            }
            OpCode::BuildTuple => {
                let v = self.pop_n(operand as usize)?;
                let val = self.heap.alloc(HeapObj::Tuple(v))?;
                self.push(val);
            }
            OpCode::BuildDict => {
                let flat = self.pop_n(operand as usize * 2)?;
                let dm = DictMap::from_pairs(flat.chunks(2).map(|c| (c[0], c[1])).collect());
                let val = self.heap.alloc(HeapObj::Dict(Rc::new(RefCell::new(dm))))?;
                self.push(val);
            }
            OpCode::BuildString => {
                let parts = self.pop_n(operand as usize)?;
                let s: String = parts.iter().map(|v| self.display(*v)).collect();
                let val = self.heap.alloc(HeapObj::Str(s))?;
                self.push(val);
            }
            OpCode::BuildSet   => self.build_set(operand)?,
            OpCode::BuildSlice => self.build_slice(operand)?,
            _ => unreachable!("non-build opcode in handle_build"),
        }
        Ok(())
    }

    /*
    Container
        Acceso/asignación indexada, unpacking y formato de valores.
    */

    pub(crate) fn handle_container(&mut self, op: OpCode, operand: u16) -> Result<(), VmErr> {
        match op {
            OpCode::GetItem => { self.get_item()?; }
            OpCode::StoreItem => {
                self.mark_impure();
                self.store_item()?;
            }
            OpCode::UnpackSequence => self.exec_unpack_seq(operand as usize)?,
            OpCode::UnpackEx => self.unpack_ex(operand)?,
            OpCode::FormatValue => {
                if operand == 1 { self.pop()?; }
                let v = self.pop()?;
                let s = self.display(v);
                let val = self.heap.alloc(HeapObj::Str(s))?;
                self.push(val);
            }
            _ => unreachable!("non-container opcode in handle_container"),
        }
        Ok(())
    }

    /*
    Comprehension
        Append/add a acumuladores en el tope de stack durante comprensiones.
    */

    pub(crate) fn handle_comprehension(&mut self, op: OpCode) -> Result<(), VmErr> {
        match op {
            OpCode::ListAppend => {
                let v = self.pop()?;
                let acc = *self.stack.last().ok_or(VmErr::Runtime("stack underflow"))?;
                if !acc.is_heap() { return Err(VmErr::Runtime("list accumulator corrupted")); }
                match self.heap.get(acc) {
                    HeapObj::List(rc) => rc.borrow_mut().push(v),
                    _ => return Err(VmErr::Runtime("list accumulator corrupted")),
                }
            }
            OpCode::SetAdd => {
                let v = self.pop()?;
                let acc = *self.stack.last().ok_or(VmErr::Runtime("stack underflow"))?;
                if !acc.is_heap() { return Err(VmErr::Runtime("set accumulator corrupted")); }
                let already = match self.heap.get(acc) {
                    HeapObj::Set(rc) => rc.borrow().iter().any(|&x| eq_vals_with_heap(x, v, &self.heap)),
                    _ => return Err(VmErr::Runtime("set accumulator corrupted")),
                };
                if !already && let HeapObj::Set(rc) = self.heap.get(acc) {
                    rc.borrow_mut().insert(v);
                }
            }
            OpCode::MapAdd => {
                let value = self.pop()?;
                let key = self.pop()?;
                let acc = *self.stack.last().ok_or(VmErr::Runtime("stack underflow"))?;
                if !acc.is_heap() { return Err(VmErr::Runtime("dict accumulator corrupted")); }
                match self.heap.get(acc) {
                    HeapObj::Dict(rc) => { rc.borrow_mut().insert(key, value); }
                    _ => return Err(VmErr::Runtime("dict accumulator corrupted")),
                }
            }
            _ => unreachable!("non-comprehension opcode in handle_comprehension"),
        }
        Ok(())
    }

    /*
    Yield
        Acumula valor en el buffer del generador y empuja None como placeholder.
    */

    pub(crate) fn handle_yield(&mut self) -> Result<(), VmErr> {
        let v = self.pop()?;
        self.yields.push(v);
        self.push(Val::none());
        Ok(())
    }

    /*
    Side
        Side-effects e impurezas: assert, del, global/nonlocal, import, type aliases, exception handling stubs y await/yield-from.
    */
    pub(crate) fn handle_side(&mut self, op: OpCode, operand: u16, slots: &mut [Option<Val>]) -> Result<(), VmErr> {
        match op {
            OpCode::Assert => {
                let v = self.pop()?;
                if !self.truthy(v) { return Err(VmErr::Runtime("AssertionError")); }
            }
            OpCode::Del => {
                let slot = operand as usize;
                if slot < slots.len() { slots[slot] = None; }
            }
            OpCode::Global | OpCode::Nonlocal => self.mark_impure(),
            OpCode::TypeAlias => { self.pop()?; }
            OpCode::Import => {
                self.mark_impure();
                self.push(Val::none());
            }
            OpCode::ImportFrom => {
                self.mark_impure();
                self.pop()?;
                self.push(Val::none());
            }
            OpCode::SetupExcept | OpCode::PopExcept => {}
            OpCode::Raise | OpCode::RaiseFrom => {
                self.mark_impure();
                return Err(VmErr::Runtime("exception raised"));
            }
            OpCode::Await | OpCode::YieldFrom => {}
            _ => unreachable!("non-side opcode in handle_side"),
        }
        Ok(())
    }
}