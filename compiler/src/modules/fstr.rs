// src/modules/fstr.rs
//
// String building and errors without core::fmt.
// Deps: itoa (ints), ryu (floats).
//
// USAGE
//   s!("x=", int n, " flags=", bool f)
//   s!(cap: 64; "prefix", str p)
//   push!(buf, int n)
//   err!("bad token '", str tok, "' at line ", int line)

#[macro_export]
macro_rules! push {
    ($s:ident, $v:literal) => { $s.push_str($v); };
    ($s:ident, str $v:expr) => { $s.push_str($v); };
    ($s:ident, int $v:expr) => {{ let mut b = itoa::Buffer::new(); $s.push_str(b.format($v)); }};
    ($s:ident, float $v:expr) => {{ let mut b = ryu::Buffer::new();  $s.push_str(b.format($v)); }};
    ($s:ident, char $v:expr) => { $s.push($v); };
    ($s:ident, bool $v:expr) => { $s.push_str(if $v { "true" } else { "false" }); };
}

#[macro_export]
macro_rules! s {
    (@b $s:ident;) => {};
    (@b $s:ident; $l:literal $(, $($r:tt)*)?) => { $s.push_str($l); $($crate::s!(@b $s; $($r)*);)? };
    (@b $s:ident; str $v:expr $(, $($r:tt)*)?) => { $s.push_str($v); $($crate::s!(@b $s; $($r)*);)? };
    (@b $s:ident; int $v:expr $(, $($r:tt)*)?) => {{ let mut _b = itoa::Buffer::new(); $s.push_str(_b.format($v)); $($crate::s!(@b $s; $($r)*);)? }};
    (@b $s:ident; float $v:expr $(, $($r:tt)*)?) => {{ let mut _b = ryu::Buffer::new(); $s.push_str(_b.format($v)); $($crate::s!(@b $s; $($r)*);)? }};
    (@b $s:ident; char $v:expr $(, $($r:tt)*)?) => { $s.push($v); $($crate::s!(@b $s; $($r)*);)? };
    (@b $s:ident; bool $v:expr $(, $($r:tt)*)?) => { $s.push_str(if $v { "true" } else { "false" }); $($crate::s!(@b $s; $($r)*);)? };

    (cap: $c:expr; $($t:tt)*) => {{ let mut _s = alloc::string::String::with_capacity($c); $crate::s!(@b _s; $($t)*); _s }};
    ($($t:tt)*) => {{ let mut _s = alloc::string::String::new(); $crate::s!(@b _s; $($t)*); _s }};
}

pub enum E {
    Parse  { ctx: &'static str },
    Custom { msg: alloc::string::String },
}

impl E {
    pub fn message(&self) -> alloc::string::String {
        match self {
            Self::Parse { ctx } => s!("parse error: ", str ctx),
            Self::Custom { msg } => msg.clone(),
        }
    }
    #[inline] pub fn parse(ctx: &'static str) -> Self { Self::Parse { ctx } }
}

impl From<E> for alloc::string::String { fn from(e: E) -> Self { e.message() } }

#[macro_export]
macro_rules! err {
    ($($t:tt)*) => { $crate::modules::fstr::E::Custom { msg: $crate::s!($($t)*) } };
}