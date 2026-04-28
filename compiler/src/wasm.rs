#[cfg(target_arch = "wasm32")]
mod runtime {
    use lol_alloc::LeakingPageAllocator;
    use crate::modules::{lexer::lexer, parser::Parser, vm::{VM, Limits, VmErr}};
    use alloc::string::String;
    use crate::s;

    #[global_allocator]
    static A: LeakingPageAllocator = LeakingPageAllocator;

    #[panic_handler]
    fn panic(_: &core::panic::PanicInfo) -> ! { core::arch::wasm32::unreachable() }

    const SZ: usize = 1 << 20;
    static mut SRC: [u8; SZ] = [0; SZ];
    static mut OUT: [u8; SZ] = [0; SZ];

    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn src_ptr() -> *mut u8 {
        core::ptr::addr_of_mut!(SRC) as *mut u8
    }

    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn out_ptr() -> *const u8 {
        core::ptr::addr_of!(OUT) as *const u8
    }

    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn run(len: usize) -> usize {
        let len = len.min(SZ);
        let src = match core::str::from_utf8(unsafe {
            core::slice::from_raw_parts(core::ptr::addr_of!(SRC) as *const u8, len)
        }) {
            Ok(s) => s,
            Err(e) => return unsafe {
                write_out(&s!("input rejected: invalid utf-8 at byte ", int e.valid_up_to()))
            },
        };

        let (mut chunk, errs) = Parser::new(src, lexer(src)).parse();

        let out: String = if !errs.is_empty() {
            let mut s = String::new();
            for (i, e) in errs.iter().enumerate() {
                if i > 0 { s.push('\n'); }
                s.push_str(&s!("syntax error at line ", int e.line + 1, ":", int e.col, ": ", str &e.msg));
            }
            s
        } else {
            crate::modules::vm::optimizer::constant_fold(&mut chunk);
            let mut vm = VM::with_limits(&chunk, Limits::sandbox());
            match vm.run() {
                Ok(_) => vm.output.join("\n"),
                Err(e) => match &e {
                    VmErr::Type(m) => s!("TypeError: ", str m),
                    VmErr::Value(m) => s!("ValueError: ", str m),
                    VmErr::Runtime(m) => s!("RuntimeError: ", str m),
                    VmErr::Name(n) => s!("NameError: '", str n, "'"),
                    VmErr::Raised(r) => s!("Exception: ", str r),
                    other => other.as_str().into(),
                }
            }
        };

        unsafe { write_out(&out) }
    }

    unsafe fn write_out(s: &str) -> usize {
        let b = s.as_bytes();
        let n = b.len().min(SZ);
        unsafe {
            core::slice::from_raw_parts_mut(core::ptr::addr_of_mut!(OUT) as *mut u8, n)
                .copy_from_slice(&b[..n]);
        }
        n
    }
}

#[cfg(all(test, feature = "wasm-tests"))]
mod tests {
    use crate::modules::{lexer::lexer, parser::Parser, vm::VM};

    #[derive(serde::Deserialize)]
    struct Case {
        src: String,
        output: Vec<String>,
        result: String,
        #[serde(default)]
        error: Option<String>,
    }

    #[test]
    fn vm_cases() {
        let cases: Vec<Case> = serde_json::from_str(
            include_str!("../tests/cases/vm.json")
        ).expect("invalid JSON");

        for case in cases {
            let (chunk, errs) = Parser::new(&case.src, lexer(&case.src)).parse();
            assert!(errs.is_empty(), "parse error on {:?}: {:?}", case.src, errs.iter().map(|e| &e.msg).collect::<Vec<_>>());

            let mut vm = VM::new(&chunk);
            match vm.run() {
                Ok(obj) => {
                    assert_eq!(vm.display(obj), case.result, "result mismatch on: {:?}", case.src);
                    assert_eq!(vm.output, case.output, "output mismatch on: {:?}", case.src);
                }
                Err(e) => match &case.error {
                    Some(expected) => assert!(
                        e.to_string().contains(expected.as_str()),
                        "wrong error on {:?}: got '{}', expected '{}'", case.src, e, expected
                    ),
                    None => panic!("VM error on {:?}: {}", case.src, e),
                }
            }
        }
    }
}