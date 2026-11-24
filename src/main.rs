//! jrust 命令行入口：`.jrust` → `.rs`。

mod ast;
mod codegen;
mod diag;
mod lexer;
mod parser;

use std::path::Path;
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
            if all_ok {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
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

fn print_usage() {
    eprintln!(
        "jrust {} —— Java 风格的 Rust 转译器（教学项目）

用法:
  jrust build <文件.jrust> [更多文件...]

说明:
  - 输入 Java 风格的 .jrust 源码，输出同名的 .rs 文件（与输入同目录）
  - 生成的 .rs 交给 rustc / cargo 编译；产物保持可读，并带 // jrust:L<行号> 源映射
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

fn print_diags(path: &Path, diags: &[diag::Diag]) {
    for d in diags {
        eprintln!("{}", d.render(&path.display().to_string()));
    }
}
