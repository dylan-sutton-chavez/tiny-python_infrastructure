// vm/handlers/attr.rs

use super::*;
use crate::modules::vm::types::BuiltinMethodId;

impl<'a> VM<'a> {
    /// Resolves `obj.name` for builtin types. Pops `obj`, pushes a bound
    /// method or returns Err if the attribute is unknown for the type.
    pub(crate) fn handle_load_attr(&mut self, name_idx: u16, chunk: &SSAChunk) -> Result<(), VmErr> {
        let name = chunk.names.get(name_idx as usize)
            .ok_or(VmErr::Runtime("LoadAttr: bad name index"))?;
        let obj = self.pop()?;

        let method_id = match (self.type_name(obj), name.as_str()) {
            ("list", "append") => BuiltinMethodId::ListAppend,
            ("dict", "keys")   => BuiltinMethodId::DictKeys,
            ("dict", "values") => BuiltinMethodId::DictValues,
            ("dict", "items")  => BuiltinMethodId::DictItems,
            (ty, attr) => {
                return Err(attr_not_found(ty, attr));
            }
        };

        let bound = self.heap.alloc(HeapObj::BoundMethod(obj, method_id))?;
        self.push(bound);
        Ok(())
    }
}

#[cold]
fn attr_not_found(ty: &str, attr: &str) -> VmErr {
    // Static message keeps VmErr::Type's &'static str contract.
    let _ = (ty, attr); // params kept for future expansion
    VmErr::Type("'object' has no attribute")
}