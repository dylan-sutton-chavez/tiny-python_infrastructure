// src/modules/fstr.rs
//
// String building, errors, and logging without core::fmt.
// Deps: itoa (ints), ryu (floats).
//
// TOKENS
//   "literal"    &str literal
//   str  expr    &str expression
//   int  expr    integer  (itoa)
//   float expr   float    (ryu)
//   char expr    char
//   bool expr    bool → "true" / "false"
//
// USAGE
//   s!("x=", int n, " flags=", bool f)
//   s!(cap: 64; "prefix", str p)
//   push!(buf, int n)
//   err!("bad token '", str tok, "' at line ", int line)
//   log_info!("processed ", int n, " items in ", float ms, "ms")
//   log_err!(e, "loading config '", str path, "'")

// push!

#[macro_export]
macro_rules! push {
    ($s:ident, $v:literal) => { $s.push_str($v); };
    ($s:ident, str $v:expr) => { $s.push_str($v); };
    ($s:ident, int $v:expr) => {{ let mut b = itoa::Buffer::new(); $s.push_str(b.format($v)); }};
    ($s:ident, float $v:expr) => {{ let mut b = ryu::Buffer::new();  $s.push_str(b.format($v)); }};
    ($s:ident, char $v:expr) => { $s.push($v); };
    ($s:ident, bool $v:expr) => { $s.push_str(if $v { "true" } else { "false" }); };
}

// s!

/// Build a `String` without `core::fmt`. See module header for token syntax.
#[macro_export]
macro_rules! s {
    // internal builder arms
    (@b $s:ident;) => {};
    (@b $s:ident; $l:literal $(, $($r:tt)*)?) => { $s.push_str($l); $($crate::s!(@b $s; $($r)*);)? };
    (@b $s:ident; str $v:expr $(, $($r:tt)*)?) => { $s.push_str($v); $($crate::s!(@b $s; $($r)*);)? };
    (@b $s:ident; int $v:expr $(, $($r:tt)*)?) => {{ let mut _b = itoa::Buffer::new(); $s.push_str(_b.format($v)); $($crate::s!(@b $s; $($r)*);)? }};
    (@b $s:ident; float $v:expr $(, $($r:tt)*)?) => {{ let mut _b = ryu::Buffer::new(); $s.push_str(_b.format($v)); $($crate::s!(@b $s; $($r)*);)? }};
    (@b $s:ident; char $v:expr $(, $($r:tt)*)?) => { $s.push($v); $($crate::s!(@b $s; $($r)*);)? };
    (@b $s:ident; bool $v:expr $(, $($r:tt)*)?) => { $s.push_str(if $v { "true" } else { "false" }); $($crate::s!(@b $s; $($r)*);)? };

    // public API
    (cap: $c:expr; $($t:tt)*) => {{ let mut _s = alloc::string::String::with_capacity($c); $crate::s!(@b _s; $($t)*); _s }};
    ($($t:tt)*) => {{ let mut _s = alloc::string::String::new(); $crate::s!(@b _s; $($t)*); _s }};
}

// Error

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

/// Construct an `E::Custom` from token syntax: `err!("unexpected '", str tok, "' at line ", int line)`
#[macro_export]
macro_rules! err {
    ($($t:tt)*) => { $crate::modules::fstr::E::Custom { msg: $crate::s!($($t)*) } };
}

// Log

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level { Debug = 0, Info = 1, Warn = 2, Error = 3 }

impl Level {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Debug => "DEBUG",
            Self::Info => "INFO ",
            Self::Warn => "WARN ",
            Self::Error => "ERROR",
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod native_log {
    use super::Level;
    type Sink = fn(Level, &str);
    fn _noop(_: Level, _: &str) {}
    static SINK: core::sync::atomic::AtomicPtr<()> = core::sync::atomic::AtomicPtr::new(_noop as *mut ());
    static MIN_LEVEL: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

    pub fn set_sink(f: Sink, min: Level) {
        SINK.store(f as *mut (), core::sync::atomic::Ordering::Relaxed);
        MIN_LEVEL.store(min as u8, core::sync::atomic::Ordering::Relaxed);
    }

    #[inline]
    pub fn emit(level: Level, msg: &str) {
        if level as u8 >= MIN_LEVEL.load(core::sync::atomic::Ordering::Relaxed) {
            let f: Sink = unsafe { core::mem::transmute(SINK.load(core::sync::atomic::Ordering::Relaxed)) };
            f(level, msg);
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub use native_log::{set_sink, emit};

#[macro_export] 
macro_rules! log { 
    ($lvl:expr, $($t:tt)*) => { 
        {
            #[cfg(not(target_arch = "wasm32"))]
            { $crate::modules::fmt::emit($lvl, &$crate::s!($($t)*)); }
        }
    }; 
}
#[macro_export] macro_rules! log_debug { ($($t:tt)*) => { $crate::log!($crate::modules::fmt::Level::Debug, $($t)*) }; }
#[macro_export] macro_rules! log_info { ($($t:tt)*) => { $crate::log!($crate::modules::fmt::Level::Info, $($t)*) }; }
#[macro_export] macro_rules! log_warn { ($($t:tt)*) => { $crate::log!($crate::modules::fmt::Level::Warn, $($t)*) }; }
#[macro_export] macro_rules! log_error { ($($t:tt)*) => { $crate::log!($crate::modules::fmt::Level::Error, $($t)*) }; }

/// Log at error level, optionally with context: `log_err!(e)` or `log_err!(e, "loading '", str path, "'")`
#[macro_export]
macro_rules! log_err {
    ($e:expr) => { 
        {
            #[cfg(not(target_arch = "wasm32"))]
            { $crate::modules::fmt::emit($crate::modules::fmt::Level::Error, &$e.message()); }
        }
    };
    ($e:expr, $($t:tt)*) => { 
        {
            #[cfg(not(target_arch = "wasm32"))]
            { $crate::modules::fmt::emit($crate::modules::fmt::Level::Error, &$crate::s!($($t)*, " — ", str &$e.message())); }
        }
    };
}

#[cfg(not(target_arch = "wasm32"))]
pub fn read_file(path: &str) -> Result<alloc::string::String, E> {
    std::fs::read_to_string(path)
        .map_err(|_| err!("io: cannot access '", str path, "'"))
}