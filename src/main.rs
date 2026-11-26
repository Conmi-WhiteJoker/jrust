//! jrust 命令行入口：`.jrust` → `.rs`，或直接解释执行 / 调试。

mod ast;
mod codegen;
mod debug;
mod diag;
mod interp;
mod json;
mod lexer;
mod parser;

use std::collections::{BTreeMap, BTreeSet};
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
                let p = Path::new(f);
                let ok = if p.is_dir() {
                    build_tree(p)
                } else {
                    build_one(p)
                };
                if !ok {
                    all_ok = false;
                }
            }
            exit_code(all_ok)
        }
        "check" => {
            if args.len() < 3 {
                print_usage();
                return ExitCode::FAILURE;
            }
            let mut all_ok = true;
            for f in &args[2..] {
                if !check_one(Path::new(f)) {
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
  jrust build <目录>                       多文件模式：按 package 生成模块树
  jrust check <文件.jrust> [更多文件...]   只做词法/语法/代码生成检查，不写文件
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

/// 递归收集目录下的 `.jrust` 源文件（跳过隐藏目录）。
fn collect_jrust(dir: &Path, out: &mut Vec<PathBuf>) {
    let rd = match std::fs::read_dir(dir) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("无法读取目录 {}: {}", dir.display(), e);
            return;
        }
    };
    for entry in rd.flatten() {
        let p = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if p.is_dir() {
            if !name.starts_with('.') {
                collect_jrust(&p, out);
            }
        } else if p.extension().and_then(|e| e.to_str()) == Some("jrust") {
            out.push(p);
        }
    }
}

/// 在生成文件的 `#![allow(...)]` 行之后插入模块声明；没有该行（如 mod.rs 汇总文件）则追加到末尾。
fn insert_mod_block(code: &mut String, block: &str) {
    if let Some(pos) = code.find("#![allow(") {
        if let Some(nl) = code[pos..].find('\n') {
            let at = pos + nl + 1;
            code.insert_str(at, block);
            return;
        }
    }
    if !code.ends_with('\n') {
        code.push('\n');
    }
    code.push_str(block);
}

/// 多文件模式构建（JR-S4/M3）：
/// - `package a.b.c;` → `a/b/c.rs`，package 最后一段必须等于文件名；
/// - 含 `main` 的文件是 crate 根，不能声明 package；
/// - 每层父模块补 `pub mod 子模块;`：父模块有源文件就写进它的产物，否则生成 `a/mod.rs`。
fn build_tree(dir: &Path) -> bool {
    let mut files = Vec::new();
    collect_jrust(dir, &mut files);
    files.sort();
    if files.is_empty() {
        eprintln!("{} 下没有 .jrust 文件", dir.display());
        return false;
    }

    struct Unit {
        prog: ast::Program,
        src_name: String,
        out_rel: PathBuf,
        has_main: bool,
    }

    let mut units: Vec<Unit> = Vec::new();
    let mut ok = true;
    for f in &files {
        let src = match std::fs::read_to_string(f) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("无法读取 {}: {}", f.display(), e);
                ok = false;
                continue;
            }
        };
        let prog = match parser::parse_file(&src) {
            Ok(p) => p,
            Err(diags) => {
                print_diags(f, &diags);
                ok = false;
                continue;
            }
        };
        let stem = f
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        let main_line = prog.classes.iter().find_map(|c| {
            c.methods
                .iter()
                .find(|m| m.is_static && m.name == "main")
                .map(|m| m.line)
        });
        let out_rel = match &prog.package {
            Some(pkg) => {
                let segs: Vec<&str> = pkg.split('.').filter(|s| !s.is_empty()).collect();
                if segs.is_empty() || segs.last() != Some(&stem.as_str()) {
                    print_diags(
                        f,
                        &[diag::Diag::error_code(
                            1,
                            "JR3029",
                            format!("package `{}` 与文件名 `{}.jrust` 不一致", pkg, stem),
                        )
                        .with_hint(
                            "package 的最后一段必须等于文件名：`package app.greet;` → `greet.jrust` → `app/greet.rs`。",
                        )],
                    );
                    ok = false;
                }
                PathBuf::from(format!("{}.rs", segs.join("/")))
            }
            None => PathBuf::from(format!("{}.rs", stem)),
        };
        if let Some(line) = main_line {
            if prog.package.is_some() {
                print_diags(
                    f,
                    &[diag::Diag::error_code(
                        line,
                        "JR3029",
                        "含 main 的文件必须是 crate 根，不能声明 package",
                    )
                    .with_hint(
                        "把 main 留在项目根目录（如 `main.jrust`），其他文件用 package 声明自己的模块路径。",
                    )],
                );
                ok = false;
            }
        } else if prog.package.is_none() {
            print_diags(
                f,
                &[diag::Diag::error_code(
                    1,
                    "JR3029",
                    "没有 package 声明的文件只能是 crate 根（含 main）",
                )
                .with_hint("给这个文件加上 `package a.b;`，或把 main 放到它里面。")],
            );
            ok = false;
        }
        units.push(Unit {
            prog,
            src_name: f.display().to_string(),
            out_rel,
            has_main: main_line.is_some(),
        });
    }
    if !ok {
        return false;
    }

    let roots: Vec<usize> = units
        .iter()
        .enumerate()
        .filter(|(_, u)| u.has_main)
        .map(|(i, _)| i)
        .collect();
    if roots.is_empty() {
        eprintln!("多文件项目里没有含 main 的 crate 根文件");
        return false;
    }
    if roots.len() > 1 {
        eprintln!("多文件项目里只能有一个含 main 的 crate 根文件");
        return false;
    }
    let root_out = units[roots[0]].out_rel.clone();

    let mut files_out: BTreeMap<PathBuf, String> = BTreeMap::new();
    let mut sources: BTreeMap<Vec<String>, PathBuf> = BTreeMap::new();
    for u in &units {
        let code = match codegen::generate_multi(&u.prog, &u.src_name) {
            Ok(c) => c,
            Err(diags) => {
                print_diags(Path::new(&u.src_name), &diags);
                ok = false;
                continue;
            }
        };
        if let Some(pkg) = &u.prog.package {
            let mp: Vec<String> = pkg
                .split('.')
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect();
            sources.insert(mp, u.out_rel.clone());
        }
        if files_out.insert(u.out_rel.clone(), code).is_some() {
            eprintln!("多个源文件生成了同一个产物 {}", u.out_rel.display());
            ok = false;
        }
    }
    if !ok {
        return false;
    }

    // 每层父模块需要声明哪些子模块。
    let mut children: BTreeMap<Vec<String>, BTreeSet<String>> = BTreeMap::new();
    for mp in sources.keys() {
        for i in 1..=mp.len() {
            children
                .entry(mp[..i - 1].to_vec())
                .or_default()
                .insert(mp[i - 1].clone());
        }
    }
    for (parent, kids) in &children {
        let target = if parent.is_empty() {
            root_out.clone()
        } else if let Some(p) = sources.get(parent) {
            p.clone()
        } else {
            let mut p = PathBuf::new();
            for seg in parent {
                p.push(seg);
            }
            p.push("mod.rs");
            p
        };
        let mut block = String::new();
        for kid in kids {
            let line = format!("pub mod {};", kid);
            let exists = files_out
                .get(&target)
                .map(|c| c.lines().any(|l| l.trim() == line))
                .unwrap_or(false);
            if !exists {
                block.push_str(&line);
                block.push('\n');
            }
        }
        if block.is_empty() {
            continue;
        }
        let entry = files_out.entry(target).or_insert_with(|| {
            format!(
                "// 由 jrust 生成：`{}` 的模块声明。\n",
                if parent.is_empty() {
                    "crate".to_string()
                } else {
                    parent.join("::")
                }
            )
        });
        insert_mod_block(entry, &block);
    }

    for (rel, content) in &files_out {
        let out = dir.join(rel);
        if let Some(d) = out.parent() {
            if let Err(e) = std::fs::create_dir_all(d) {
                eprintln!("无法创建 {}: {}", d.display(), e);
                ok = false;
                continue;
            }
        }
        if let Err(e) = std::fs::write(&out, content) {
            eprintln!("无法写入 {}: {}", out.display(), e);
            ok = false;
            continue;
        }
        println!("已生成 {}", out.display());
    }
    ok
}

fn check_one(path: &Path) -> bool {
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
        Ok(_) => {
            println!("检查通过 {}", path.display());
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
