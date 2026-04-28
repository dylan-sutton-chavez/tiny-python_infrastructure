extern crate alloc;

use compiler_lib::modules::{lexer::lexer, parser::Parser, vm::{VM, Limits}};
use std::{env, fs, process::exit};
use compiler_lib::s;

const HELP: &str = "
usage: edge [options] <file>
       edge -c <code>

options:
  -c <code>    run inline code
  -d           debug output (verbosity level 1)
  -dd          debug output (verbosity level 2)
  -q           suppress info logs
  --sandbox    enable limits
  -h           show this help
";

#[inline]
fn eprint_msg(msg: &str) {
    use std::io::Write;
    let _ = writeln!(std::io::stderr(), "{}", msg);
}

#[inline]
fn print_msg(level: &str, msg: &str) {
    use std::io::Write;
    let _ = writeln!(std::io::stdout(), "[{}] {}", level, msg);
}

fn parse_args() -> (String, usize, bool, bool) {
    let args: Vec<_> = env::args().skip(1).collect();
    if args.is_empty() || args.iter().any(|a| a == "-h") {
        print!("{}", HELP);
        exit(0);
    }
    let q = args.iter().any(|a| a == "-q");
    let sandbox = args.iter().any(|a| a == "--sandbox");
    let v = args.iter().filter(|&a| a == "-dd").count() * 2 + args.iter().filter(|&a| a == "-d").count();

    if let Some(pos) = args.iter().position(|a| a == "-c") {
        let code = args.get(pos + 1).cloned().unwrap_or_default();
        return (code, v, q, sandbox);
    }
    let p = args.iter().find(|&a| !a.starts_with('-')).cloned().unwrap_or_else(|| {
        eprint_msg("abort: no input file specified");
        exit(1);
    });
    (p, v, q, sandbox)
}

fn run(path: &str, sandbox: bool, verbosity: usize, quiet: bool) -> Result<(), String> {
    let src = if path.ends_with(".py") {
        fs::read_to_string(path).map_err(|_| s!("io: cannot access '", str path, "'"))?
    } else {
        path.to_string()
    };

    let (mut chunk, errs) = Parser::new(&src, lexer(&src)).parse();
    if !errs.is_empty() {
        for e in &errs {
            eprint_msg(&s!("syntax: ", str &e.render_with_path(path)));
        }
        exit(1);
    }
    compiler_lib::modules::vm::optimizer::constant_fold(&mut chunk);

    if !quiet {
        print_msg("info", &s!(
            "emit: snapshot created [ops=", int chunk.instructions.len(), " consts=", int chunk.constants.len(), "]"));
    }

    let limits = if sandbox { Limits::sandbox() } else { Limits::none() };
    let mut vm = VM::with_limits(&chunk, limits);
    let exec_result = vm.run();

    vm.output.iter().for_each(|l| println!("{l}"));

    if let Err(e) = exec_result {
        return Err(e.render());
    }

    if verbosity >= 1 {
        let (sp, tot) = vm.cache_stats();
        print_msg("debug", &s!(
            "vm: specialization_ratio=", int sp, "/", int tot, " [heap_footprint=", int vm.heap_usage(), "b]"));
    }

    Ok(())
}

fn main() {
    let (p, v, q, sandbox) = parse_args();
    if let Err(e) = run(&p, sandbox, v, q) {
        eprint_msg(&e);
        exit(1);
    }
}