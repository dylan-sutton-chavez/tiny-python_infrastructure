use super::*;
use crate::modules::vm::types::BuiltinMethodId;

type AttrRow = (&'static str, &'static str, BuiltinMethodId);

// Table of built-in methods organized by (type, attribute_name). It must be kept in lexicographical order to allow binary search.
static ATTR_TABLE: &[AttrRow] = &[
    ("dict", "get", BuiltinMethodId::DictGet),
    ("dict", "items", BuiltinMethodId::DictItems),
    ("dict", "keys", BuiltinMethodId::DictKeys),
    ("dict", "pop", BuiltinMethodId::DictPop),
    ("dict", "setdefault", BuiltinMethodId::DictSetDefault),
    ("dict", "update", BuiltinMethodId::DictUpdate),
    ("dict", "values", BuiltinMethodId::DictValues),
    ("list", "append", BuiltinMethodId::ListAppend),
    ("list", "clear", BuiltinMethodId::ListClear),
    ("list", "copy", BuiltinMethodId::ListCopy),
    ("list", "count", BuiltinMethodId::ListCount),
    ("list", "extend", BuiltinMethodId::ListExtend),
    ("list", "index", BuiltinMethodId::ListIndex),
    ("list", "insert", BuiltinMethodId::ListInsert),
    ("list", "pop", BuiltinMethodId::ListPop),
    ("list", "remove", BuiltinMethodId::ListRemove),
    ("list", "reverse", BuiltinMethodId::ListReverse),
    ("list", "sort", BuiltinMethodId::ListSort),
    ("str", "capitalize", BuiltinMethodId::StrCapitalize),
    ("str", "center", BuiltinMethodId::StrCenter),
    ("str", "count", BuiltinMethodId::StrCount),
    ("str", "endswith", BuiltinMethodId::StrEndswith),
    ("str", "find", BuiltinMethodId::StrFind),
    ("str", "isalnum", BuiltinMethodId::StrIsAlnum),
    ("str", "isalpha", BuiltinMethodId::StrIsAlpha),
    ("str", "isdigit", BuiltinMethodId::StrIsDigit),
    ("str", "join", BuiltinMethodId::StrJoin),
    ("str", "lower", BuiltinMethodId::StrLower),
    ("str", "lstrip", BuiltinMethodId::StrLstrip),
    ("str", "replace", BuiltinMethodId::StrReplace),
    ("str", "rstrip", BuiltinMethodId::StrRstrip),
    ("str", "split", BuiltinMethodId::StrSplit),
    ("str", "startswith", BuiltinMethodId::StrStartswith),
    ("str", "strip", BuiltinMethodId::StrStrip),
    ("str", "title", BuiltinMethodId::StrTitle),
    ("str", "upper", BuiltinMethodId::StrUpper),
    ("str", "zfill", BuiltinMethodId::StrZfill),
];

#[inline]
fn lookup_attr(ty: &str, attr: &str) -> Option<BuiltinMethodId> {
    ATTR_TABLE.binary_search_by(|&(t, a, _)| { t.cmp(ty).then_with(|| a.cmp(attr))})
        .ok()
        .map(|i| ATTR_TABLE[i].2)
}

impl<'a> VM<'a> {
    pub(crate) fn handle_load_attr(&mut self, name_idx: u16, chunk: &SSAChunk) -> Result<(), VmErr> {
        let name = chunk.names.get(name_idx as usize).ok_or(VmErr::Runtime("LoadAttr: bad name index"))?;
        
        let obj = self.pop()?;
        let ty = self.type_name(obj);

        // Binary search using O(log n)
        let method_id = lookup_attr(ty, name.as_str()).ok_or_else(|| attr_not_found(ty, name.as_str()))?;

        let bound = self.heap.alloc(HeapObj::BoundMethod(obj, method_id))?;
        self.push(bound);
        
        Ok(())
    }
}

#[cold]
fn attr_not_found(ty: &str, attr: &str) -> VmErr {
    let _ = (ty, attr); 
    VmErr::Type("'object' has no attribute")
}