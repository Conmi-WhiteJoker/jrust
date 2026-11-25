//! jrust 命令行入口：`.jrust` → `.rs`，或直接解释执行 / 调试。

mod ast;
mod codegen;
mod debug;
mod diag;
mod interp;
mod json;
mod lexer;
mod parser;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        print_usage();
        return ExitCode::FAILURE;
    }
    match args[1].as_str() {
        "build" => {
            if args.len() < 3 {
                print_usage();
                return ExitCode::FAILURE;
            }
            let mut all_ok = true;
            for f in &args[2..] {
                if !build_one(Path::new(f)) {
                    all_ok = false;
                }
            }
            exit_code(all_ok)
        }
        "run" => {
            if args.len() < 3 {
                print_usage();
                return ExitCode::FAILURE;
            }
            let mut all_ok = true;
            for f in &args[2..] {
                if !run_one(Path::new(f)) {
                    all_ok = false;
                }
            }
            exit_code(all_ok)
        }
        "debug" => {
            if args.len() < 3 {
                print_usage();
                return ExitCode::FAILURE;
            }
            exit_code(debug_one(Path::new(&args[2])))
        }
        "dap" => dap_main(),
        "-h" | "--help" | "help" => {
            print_usage();
            ExitCode::SUCCESS
        }
        "version" | "-V" | "--version" => {
            println!("jrust {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        other => {
            eprintln!("未知子命令: {}", other);
            print_usage();
            ExitCode::FAILURE
        }
    }
}

fn exit_code(ok: bool) -> ExitCode {
    if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn print_usage() {
    eprintln!(
        "jrust {} —— Java 风格的 Rust 转译器（教学项目）

用法:
  jrust build <文件.jrust> [更多文件...]   转译成同名 .rs
  jrust run <文件.jrust>                   直接解释执行（不经过 rustc）
  jrust debug <文件.jrust>                 命令行调试（断点按 .jrust 行号）
  jrust dap                                以 DAP 适配器方式运行（供 IDE 连接）

说明:
  - 输入 Java 风格的 .jrust 源码，输出同名的 .rs 文件（与输入同目录）
  - 生成的 .rs 交给 rustc / cargo 编译；产物保持可读，并带 // jrust:L<行号> 源映射
  - `run` / `debug` 使用内置树遍历解释器，断点与变量都作用在 .jrust 源码行上
  - 遇到 jrust 砍掉的 Java 特性时，会解释 Rust 为什么这么要求",
        env!("CARGO_PKG_VERSION")
    );
}

fn build_one(path: &Path) -> bool {
    let src = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("无法读取 {}: {}", path.display(), e);
            return false;
        }
    };
    let prog = match parser::parse_file(&src) {
        Ok(p) => p,
        Err(diags) => {
            print_diags(path, &diags);
            return false;
        }
    };
    match codegen::generate(&prog, &path.display().to_string()) {
        Ok(code) => {
            let out = path.with_extension("rs");
            if let Err(e) = std::fs::write(&out, code) {
                eprintln!("无法写入 {}: {}", out.display(), e);
                return false;
            }
            println!("已生成 {}", out.display());
            true
        }
        Err(diags) => {
            print_diags(path, &diags);
            false
        }
    }
}

fn load_program(path: &Path) -> Option<(String, ast::Program)> {
    let src = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("无法读取 {}: {}", path.display(), e);
            return None;
        }
    };
    match parser::parse_file(&src) {
        Ok(p) => Some((src, p)),
        Err(diags) => {
            print_diags(path, &diags);
            None
        }
    }
}

fn run_one(path: &Path) -> bool {
    let (_, prog) = match load_program(path) {
        Some(v) => v,
        None => return false,
    };
    let mut it = interp::Interp::new(&prog, &path.display().to_string());
    match it.run_main() {
        Ok(()) => true,
        Err(e) => {
            if !e.quit {
                eprintln!("{}", e.render(&path.display().to_string()));
            }
            false
        }
    }
}

fn debug_one(path: &Path) -> bool {
    let display = path.display().to_string();
    let (src, prog) = match load_program(path) {
        Some(v) => v,
        None => return false,
    };
    let lines: Vec<String> = src.lines().map(|l| l.to_string()).collect();
    println!("jrust 调试器 —— 断点按 .jrust 行号。输入 help 查看命令，run 开始执行。");
    let mut it = interp::Interp::new(&prog, &display);
    it.debug = Some(debug::Debugger::cli(&display, lines));
    let mut code = 0;
    if let Err(e) = it.run_main() {
        if !e.quit {
            eprintln!("{}", e.render(&display));
            code = 1;
        }
    }
    if let Some(mut dbg) = it.take_debug() {
        dbg.on_finish(code);
    }
    code == 0
}

fn dap_main() -> ExitCode {
    let mut state = debug::DapState::new();
    let program = match state.handshake() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("DAP: {}", e);
            return ExitCode::FAILURE;
        }
    };
    let path = PathBuf::from(&program);
    let (src, prog) = match load_program(&path) {
        Some(v) => v,
        None => {
            state.finish(1);
            return ExitCode::FAILURE;
        }
    };
    let _ = src;
    let mut it = interp::Interp::new(&prog, &program);
    it.debug = Some(debug::Debugger::Dap(state));
    let code = match it.run_main() {
        Ok(()) => 0,
        Err(e) => {
            if !e.quit {
                eprintln!("{}", e.render(&program));
                1
            } else {
                0
            }
        }
    };
    if let Some(mut dbg) = it.take_debug() {
        dbg.on_finish(code);
    }
    exit_code(code == 0)
}

fn print_diags(path: &Path, diags: &[diag::Diag]) {
    for d in diags {
        eprintln!("{}", d.render(&path.display().to_string()));
    }
}
