// src/modules/fstr.rs
//
// String building and errors without core::fmt.
// Deps: itoa (ints).

/// Format an f64 to a string suitable for Python display.
pub fn format_f64(f: f64) -> alloc::string::String {
    if f != f { return alloc::string::String::from("NaN"); }
    if f == f64::INFINITY { return alloc::string::String::from("inf"); }
    if f == f64::NEG_INFINITY { return alloc::string::String::from("-inf"); }
    if f == 0.0 {
        return if f.is_sign_negative() { alloc::string::String::from("-0.0") }
               else { alloc::string::String::from("0.0") };
    }

    // Whole-number floats: use itoa + ".0"
    const I64_UPPER: f64 = i64::MAX as f64;
    if f.is_finite() && f >= (i64::MIN as f64) && f < I64_UPPER && f == (f as i64) as f64 {
        let mut b = itoa::Buffer::new();
        let s = b.format(f as i64);
        let mut out = alloc::string::String::with_capacity(s.len() + 2);
        out.push_str(s);
        out.push_str(".0");
        return out;
    }

    format_general(f)
}

fn format_general(f: f64) -> alloc::string::String {
    // Use the standard Rust float formatting via a small stack buffer.
    // We write into a fixed buffer using core::fmt::Write.
    let mut buf = FmtBuf::new();
    let _ = core::fmt::write(&mut buf, core::format_args!("{}", f));
    alloc::string::String::from(buf.as_str())
}

struct FmtBuf { buf: [u8; 32], len: usize }
impl FmtBuf {
    fn new() -> Self { Self { buf: [0u8; 32], len: 0 } }
    fn as_str(&self) -> &str {
        unsafe { core::str::from_utf8_unchecked(&self.buf[..self.len]) }
    }
}
impl core::fmt::Write for FmtBuf {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let bytes = s.as_bytes();
        let end = (self.len + bytes.len()).min(self.buf.len());
        let n = end - self.len;
        self.buf[self.len..end].copy_from_slice(&bytes[..n]);
        self.len = end;
        Ok(())
    }
}

#[macro_export]
macro_rules! push {
    ($s:ident, $v:literal) => { $s.push_str($v); };
    ($s:ident, str $v:expr) => { $s.push_str($v); };
    ($s:ident, int $v:expr) => {{ let mut b = itoa::Buffer::new(); $s.push_str(b.format($v)); }};
    ($s:ident, float $v:expr) => { $s.push_str(&$crate::modules::fstr::format_f64($v)); };
    ($s:ident, char $v:expr) => { $s.push($v); };
    ($s:ident, bool $v:expr) => { $s.push_str(if $v { "true" } else { "false" }); };
}

#[macro_export]
macro_rules! s {
    (@b $s:ident;) => {};
    (@b $s:ident; $l:literal $(, $($r:tt)*)?) => { $s.push_str($l); $($crate::s!(@b $s; $($r)*);)? };
    (@b $s:ident; str $v:expr $(, $($r:tt)*)?) => { $s.push_str($v); $($crate::s!(@b $s; $($r)*);)? };
    (@b $s:ident; int $v:expr $(, $($r:tt)*)?) => {{ let mut _b = itoa::Buffer::new(); $s.push_str(_b.format($v)); $($crate::s!(@b $s; $($r)*);)? }};
    (@b $s:ident; float $v:expr $(, $($r:tt)*)?) => { $s.push_str(&$crate::modules::fstr::format_f64($v)); $($crate::s!(@b $s; $($r)*);)? };
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
