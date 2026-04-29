// parser/types.rs

use crate::s;
use crate::modules::fx::FxHashMap as HashMap;

use alloc::{string::{String, ToString}, vec, vec::Vec};

pub(crate) const MAX_EXPR_DEPTH: usize = 200;
pub(crate) const MAX_INSTRUCTIONS: usize = 65_535;

/* Enumeration of all bytecode instructions supported by the virtual machine. */

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OpCode { 
    LoadConst, LoadName, StoreName, Call, PopTop, ReturnValue, BuildString, CallPrint, CallLen, 
    FormatValue, CallAbs, Minus, CallStr, CallInt, CallRange, Phi, CallChr, CallType, MakeFunction, 
    Add, Sub, Mul, Div, Eq, CallFloat, CallBool, CallRound, CallMin, CallMax, CallSum, CallSorted, 
    CallEnumerate, CallZip, CallList, CallTuple, CallDict, CallIsInstance, CallSet, CallInput, 
    CallOrd, BuildDict, BuildList, NotEq, Lt, Gt, LtEq, GtEq, And, Or, Not, JumpIfFalse, Jump, 
    GetIter, ForIter, GetItem, Mod, Pow, FloorDiv, LoadTrue, LoadFalse, LoadNone, LoadAttr, StoreAttr, 
    BuildSlice, MakeClass, SetupExcept, PopExcept, Raise, Import, ImportFrom, BitAnd, BitOr, BitXor, 
    BitNot, Shl, Shr, In, NotIn, Is, IsNot, UnpackSequence, BuildTuple, SetupWith, ExitWith, Yield, 
    Del, Assert, Global, Nonlocal, UnpackArgs, ListAppend, SetAdd, MapAdd, BuildSet, RaiseFrom, 
    UnpackEx, LoadEllipsis, Await, MakeCoroutine, YieldFrom, TypeAlias, StoreItem, Dup2, 
    JumpIfFalseOrPop, JumpIfTrueOrPop, Dup,
}

/* O(1) lookup table mapping Python builtin names to their corresponding OpCodes. */

pub(super) fn builtin(name: &str) -> Option<(OpCode, bool)> {
    match name {
        "len" => Some((OpCode::CallLen, true)),
        "abs" => Some((OpCode::CallAbs, true)),
        "str" => Some((OpCode::CallStr, true)),
        "int" => Some((OpCode::CallInt, true)),
        "type" => Some((OpCode::CallType, true)),
        "float" => Some((OpCode::CallFloat, true)),
        "bool" => Some((OpCode::CallBool, true)),
        "round" => Some((OpCode::CallRound, true)),
        "min" => Some((OpCode::CallMin, true)),
        "max" => Some((OpCode::CallMax, true)),
        "sum" => Some((OpCode::CallSum, true)),
        "sorted" => Some((OpCode::CallSorted, true)),
        "enumerate" => Some((OpCode::CallEnumerate, true)),
        "zip" => Some((OpCode::CallZip, true)),
        "list" => Some((OpCode::CallList, true)),
        "tuple" => Some((OpCode::CallTuple, true)),
        "dict" => Some((OpCode::CallDict, true)),
        "set" => Some((OpCode::CallSet, true)),
        "input" => Some((OpCode::CallInput, true)),
        "isinstance" => Some((OpCode::CallIsInstance, true)),
        "chr" => Some((OpCode::CallChr, true)),
        "ord" => Some((OpCode::CallOrd, true)),
        _ => None,
    }
}

/* Represents constant literals stored in the bytecode constants pool. */

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Str(String),
    Int(i64),
    BigInt(String),
    Float(f64),
    Bool(bool),
    None,
}

/* Single bytecode instruction containing an opcode and 16-bit operand. */

#[derive(Debug, Clone, Copy)]
pub struct Instruction {
    pub opcode: OpCode,
    pub operand: u16,
}

/* Container for generated instructions, constants, names, PHI sources and metadata. */

#[derive(Default, Clone)]
pub struct SSAChunk {
    pub instructions: Vec<Instruction>,
    pub constants: Vec<Value>,
    pub names: Vec<String>,
    pub functions: Vec<(Vec<String>, SSAChunk, u16, u16)>,
    pub annotations: HashMap<String, String>,
    pub phi_sources: Vec<(u16, u16)>,
    pub classes: Vec<SSAChunk>,
    pub is_pure: bool,
    pub overflow: bool,
    pub prev_slots: Vec<Option<u16>>,
    pub phi_map: Vec<usize>,
    pub nonlocals: Vec<String>,
    pub(super) name_index: HashMap<String, u16>,
}

impl SSAChunk {
    pub(super) fn emit(&mut self, op: OpCode, operand: u16) {
        // Sets overflow flag for post parse diagnostic instead of panicking
        if self.instructions.len() >= MAX_INSTRUCTIONS {
            self.overflow = true;
            return;
        }
        self.instructions.push(Instruction { opcode: op, operand });
    }

    pub(super) fn push_const(&mut self, v: Value) -> u16 {
        if self.constants.len() >= u16::MAX as usize {
            return 0;
        }
        self.constants.push(v);
        (self.constants.len() - 1) as u16
    }

    pub(super) fn push_name(&mut self, n: &str) -> u16 {
        if let Some(&i) = self.name_index.get(n) { return i; }
        if self.names.len() >= u16::MAX as usize {
            return 0;
        }
        let i = self.names.len() as u16;
        self.names.push(n.to_string());
        self.name_index.insert(n.to_string(), i);
        i
    }

    pub fn finalize_prev_slots(&mut self) {
        let mut ps: Vec<Option<u16>> = vec![None; self.names.len()];
        for (i, name) in self.names.iter().enumerate() {
            if let Some(pos) = name.rfind('_')
                && let Ok(ver) = name[pos+1..].parse::<u32>()
                && ver > 0 {
                    let prev = s!(str &name[..pos], "_", int ver - 1);
                    if let Some(&j) = self.name_index.get(&prev) {
                        ps[i] = Some(j);
                    }
            }
        }
        self.prev_slots = ps;

        for (_, body, _, _) in &mut self.functions {
            body.finalize_prev_slots();
        }

        let phi_count = self.instructions.iter().filter(|i| i.opcode == OpCode::Phi).count();
        if phi_count > 0 {
            self.phi_map = vec![0; self.instructions.len()];
            let mut phi_idx = 0;
            for (i, ins) in self.instructions.iter().enumerate() {
                if ins.opcode == OpCode::Phi {
                    self.phi_map[i] = phi_idx;
                    phi_idx += 1;
                }
            }
        }
    }
}

/* Tracks SSA versions before/after branches to insert correct PHI nodes later. */

pub(crate) struct JoinNode {
    pub(super) backup: HashMap<String, u32>,
    pub(super) then: Option<HashMap<String, u32>>,
}

/* Stores parsing error details including line, column range and message. */

pub struct Diagnostic {
    pub line: usize,
    pub col: usize,
    pub end: usize,
    pub msg: String,
}

#[cfg(not(target_arch = "wasm32"))]
impl Diagnostic {
    pub fn render(&self) -> alloc::string::String {
        crate::s!("line ", int self.line + 1, ":", int self.col, ": ", str &self.msg)
    }

    pub fn render_with_path(&self, path: &str) -> alloc::string::String {
        crate::s!(str path, ":", int self.line + 1, ":", int self.col, ": ", str &self.msg)
    }
}

/* Parses and unescapes Python string literals from lexer tokens. */

pub(super) fn parse_string(s: &str) -> String {
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

    let take_hex = |chars: &mut core::iter::Peekable<core::str::Chars>, n: usize| -> char {
        let hex: String = chars.by_ref().take(n).collect();
        u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32).unwrap_or('\u{FFFD}')
    };

    while let Some(c) = chars.next() {
        if c != '\\' { out.push(c); continue; }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('\\') => out.push('\\'),
            Some('\'') => out.push('\''),
            Some('"') => out.push('"'),
            Some('x') => out.push(take_hex(&mut chars, 2)),
            Some('u') => out.push(take_hex(&mut chars, 4)),
            Some('U') => out.push(take_hex(&mut chars, 8)),
            Some('0') => out.push('\0'),
            Some(c) => { out.push('\\'); out.push(c); }
            None => out.push('\\'),
        }
    }
    out
}

// Array containing the default data types for the language.
pub const BUILTIN_TYPES: &[&str] = &[
    "int", "float", "str", "bool", "list",
    "tuple", "dict", "set", "range", "type", "NoneType",
    "Exception", "BaseException",
    "ValueError", "TypeError", "NameError", "KeyError",
    "IndexError", "AttributeError", "RuntimeError",
    "ZeroDivisionError", "OverflowError", "MemoryError",
    "RecursionError", "StopIteration", "NotImplementedError",
    "OSError", "IOError", "ImportError", "ModuleNotFoundError",
    "AssertionError", "ArithmeticError", "LookupError",
];

/* Map each opcode to its functional group for streamlined execution logic. */

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OpCategory {
    Load, Store,
    Arith, Bitwise, Compare, Logic, Identity,
    ControlFlow, Iter,
    Build, Container, Comprehension,
    Function, Ssa, Yield, Side,
    Unsupported,
}

impl OpCode {
    pub fn category(self) -> OpCategory {
        use OpCode::*;
        match self {
            LoadConst | LoadName | LoadTrue | LoadFalse | LoadNone | LoadEllipsis => OpCategory::Load, StoreName => OpCategory::Store,
            Add | Sub | Mul | Div | Mod | Pow | FloorDiv | Minus => OpCategory::Arith, BitAnd | BitOr | BitXor | BitNot | Shl | Shr => OpCategory::Bitwise,
            Eq | NotEq | Lt | Gt | LtEq | GtEq => OpCategory::Compare, And | Or | Not => OpCategory::Logic,
            In | NotIn | Is | IsNot => OpCategory::Identity, 
            Jump | JumpIfFalse | JumpIfFalseOrPop | JumpIfTrueOrPop | ReturnValue | PopTop | Dup | Dup2 => OpCategory::ControlFlow,
            GetIter | ForIter => OpCategory::Iter,
            BuildList | BuildTuple | BuildDict | BuildSet | BuildSlice | BuildString => OpCategory::Build,
            GetItem | StoreItem | UnpackSequence | UnpackEx | FormatValue => OpCategory::Container,
            ListAppend | SetAdd | MapAdd => OpCategory::Comprehension,
            Call | MakeFunction | MakeCoroutine | CallPrint | CallLen | CallAbs | CallStr | CallInt | CallRange | CallChr | CallType | CallFloat | CallBool | CallRound | CallMin | CallMax | CallSum | CallSorted | CallEnumerate | CallZip | CallList | CallTuple | CallDict | CallIsInstance | CallSet | CallInput | CallOrd => OpCategory::Function,
            Phi => OpCategory::Ssa,
            Yield => OpCategory::Yield,
            Assert | Del | Global | Nonlocal | TypeAlias | Import | ImportFrom | SetupExcept | PopExcept | Raise | RaiseFrom | Await | YieldFrom => OpCategory::Side,
            LoadAttr => OpCategory::Load,
            SetupWith | ExitWith => OpCategory::ControlFlow,
            UnpackArgs => OpCategory::Container,
            MakeClass | StoreAttr => OpCategory::Unsupported,
        }
    }
}