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
        "new" => {
            if args.len() != 3 {
                print_usage();
                return ExitCode::FAILURE;
            }
            exit_code(new_project(Path::new(&args[2])))
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
  jrust new <目录>                         新建项目（main.jrust + main.rs + Cargo.toml，可直接 cargo run）
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

/// `jrust new` 写出的项目骨架：`main.jrust`（源）+ 即时转译的 `main.rs` + `Cargo.toml`。
/// 源文件名固定为 `main.jrust`：OpenIDEA 插件的运行配置以它作为项目触发标记，
/// 类名与文件名不必一致。`Cargo.toml` 用 `[[bin]] path` 直接指向转译产物，
/// 没有 `src/` 目录也能 `cargo run`。
const NEW_PROJECT_MAIN: &str = r#"public class Main {
    public static void main(String[] args) {
        System.out.println("Hello, jrust!");
    }
}
"#;

fn new_project(dir: &Path) -> bool {
    if dir.exists() {
        eprintln!("{} 已存在，请换一个名称或先删除它", dir.display());
        return false;
    }
    if let Err(e) = std::fs::create_dir_all(dir) {
        eprintln!("无法创建 {}: {}", dir.display(), e);
        return false;
    }
    let main = dir.join("main.jrust");
    if let Err(e) = std::fs::write(&main, NEW_PROJECT_MAIN) {
        eprintln!("无法写入 {}: {}", main.display(), e);
        return false;
    }
    if !build_one(&main) {
        return false;
    }
    let name = crate_name(dir);
    let manifest = format!(
        "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[[bin]]\nname = \"{name}\"\npath = \"main.rs\"\n"
    );
    let cargo = dir.join("Cargo.toml");
    if let Err(e) = std::fs::write(&cargo, manifest) {
        eprintln!("无法写入 {}: {}", cargo.display(), e);
        return false;
    }
    println!(
        "已创建 jrust 项目 {}（main.jrust / main.rs / Cargo.toml）",
        dir.display()
    );
    true
}

/// 目录名 → 合法的 crate 名：只保留 ASCII 字母数字与 `-`/`_`，其余替换为 `_`；
/// 不能以数字或符号开头（Cargo 的限制），必要时加 `app-` 前缀。
fn crate_name(dir: &Path) -> String {
    let raw = dir.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let mut name: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if name.is_empty() {
        name.push_str("jrust-app");
    } else if !name.starts_with(|c: char| c.is_ascii_alphabetic()) {
        name.insert_str(0, "app-");
    }
    name
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
