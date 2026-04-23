use compiler_lib::modules::{lexer::lexer, parser::Parser, vm::{VM, Limits}};
use std::{env, fs, process::exit};
use log::{debug, info, error};

const HELP: &str = "
usage: edge [options] <file>
       edge -c <code>

options:
  -c <code>    run inline code
  -d           debug output (verbosity level 1)
  -dd          debug output (verbosity level 2)
  -q           suppress info logs (errors still shown)
  --sandbox    enable limits (512 calls, 1e8 ops, 1e5 heap)
  -h           show this help
";

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
        error!("abort: no input file specified");
        exit(1);
    });
    (p, v, q, sandbox)
}

fn run(path: &str, sandbox: bool) -> Result<(), String> {
    let src = if path.ends_with(".py") {
        fs::read_to_string(path).map_err(|e| format!("io: cannot access '{}': {}", path, e))?
    } else {
        path.to_string()
    };

    let (chunk, errs) = Parser::new(&src, lexer(&src)).parse();
    if !errs.is_empty() {
        for e in &errs {
            error!("syntax: {}:{}:{}: {}", path, e.line + 1, e.col, e.msg);
        }
        exit(1);
    }

    info!("emit: snapshot created [ops={} consts={}]", chunk.instructions.len(), chunk.constants.len());

    let limits = if sandbox { Limits::sandbox() } else { Limits::none() };
    let mut vm = VM::with_limits(&chunk, limits);
    let exec_result = vm.run();

    vm.output.iter().for_each(|l| println!("{l}"));

    if let Err(e) = exec_result {
        return Err(e.to_string());
    }

    let (sp, tot) = vm.cache_stats();
    debug!("vm: specialization_ratio={}/{} [heap_footprint={}b]", sp, tot, vm.heap_usage());

    Ok(())
}

fn main() {
    let (p, v, q, sandbox) = parse_args();

    let default_level = match (q, v) {
        (true, _) => "error",
        (_, 0) => "info",
        (_, 1) => "debug",
        _ => "trace",
    };
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or(default_level)
    ).init();

    if let Err(e) = run(&p, sandbox) {
        error!("{}", e);
        exit(1);
    }
}