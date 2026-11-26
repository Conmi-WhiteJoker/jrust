//! jrust 命令行入口：`.jrust` → `.rs`，或直接解释执行 / 调试。

mod analysis;
mod ast;
mod codegen;
mod debug;
mod diag;
mod interp;
mod json;
mod lexer;
mod parser;

use std::collections::{BTreeMap, BTreeSet, HashSet};
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
        "fmt" => {
            if args.len() < 3 {
                print_usage();
                return ExitCode::FAILURE;
            }
            let mut all_ok = true;
            for a in &args[2..] {
                let p = Path::new(a);
                if p.is_dir() {
                    let mut files = Vec::new();
                    collect_jrust(p, &mut files);
                    files.sort();
                    for f in &files {
                        if !fmt_one(f) {
                            all_ok = false;
                        }
                    }
                } else if !fmt_one(p) {
                    all_ok = false;
                }
            }
            exit_code(all_ok)
        }
        "verify" => {
            if args.len() < 3 {
                exit_code(verify_suite())
            } else {
                let mut all_ok = true;
                for f in &args[2..] {
                    if !verify_one(Path::new(f)) {
                        all_ok = false;
                    }
                }
                exit_code(all_ok)
            }
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
  jrust verify [文件.jrust...]             转译 + rustc 编译校验；不带参数时跑一致性套件
  jrust fmt <文件或目录>                   格式化 .jrust（统一 4 空格缩进，raw 块原样保留）
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

/// 从源文件所在目录向上找最近的 `Cargo.toml`（JR-M4：依赖的唯一事实来源）。
fn find_manifest(path: &Path) -> Option<PathBuf> {
    let mut dir = path.parent()?.to_path_buf();
    loop {
        let candidate = dir.join("Cargo.toml");
        if candidate.is_file() {
            return Some(candidate);
        }
        if !dir.pop() {
            return None;
        }
    }
}

/// 读取 `Cargo.toml` 里声明的依赖名（`-` 归一成 `_`，与 Rust crate 名一致）。
fn manifest_deps(manifest: &Path) -> Vec<String> {
    let text = std::fs::read_to_string(manifest).unwrap_or_default();
    let mut deps = Vec::new();
    let mut section = String::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            section = line[1..line.len() - 1].trim().to_string();
            for key in [
                "dependencies.",
                "dev-dependencies.",
                "build-dependencies.",
            ] {
                if let Some(rest) = section.strip_prefix(key) {
                    let name = rest.trim().trim_matches('"').trim_matches('\'');
                    if !name.is_empty() {
                        deps.push(name.replace('-', "_"));
                    }
                }
            }
            continue;
        }
        if matches!(
            section.as_str(),
            "dependencies" | "dev-dependencies" | "build-dependencies"
        ) {
            if let Some((k, _)) = line.split_once('=') {
                let name = k.trim().trim_matches('"').trim_matches('\'');
                if !name.is_empty() {
                    deps.push(name.replace('-', "_"));
                }
            }
        }
    }
    deps
}

/// 源文件是否位于 manifest 同级的 `src/` 目录下（JR-M5 的工程目录约定）。
fn is_under_src(src: &Path, manifest: &Path) -> bool {
    let src = match std::fs::canonicalize(src) {
        Ok(p) => p,
        Err(_) => return false,
    };
    let root = match manifest.parent().and_then(|p| std::fs::canonicalize(p).ok()) {
        Some(p) => p,
        None => return false,
    };
    match src.strip_prefix(&root) {
        Ok(rel) => rel
            .components()
            .next()
            .map(|c| c.as_os_str() == "src")
            .unwrap_or(false),
        Err(_) => false,
    }
}

/// JR-M4 / JR-C15：`.jrust` 里用到的外部 crate 必须在最近的 `Cargo.toml` 里声明，
/// async 代码还要求 tokio；缺失报 `JR5001`（jrust 只读校验，不生成也不改写 Cargo.toml）。
fn validate_deps(
    src: &Path,
    prog: &ast::Program,
    local_roots: &HashSet<String>,
) -> Vec<diag::Diag> {
    let mut out = Vec::new();
    let mut external: Vec<&str> = Vec::new();
    for imp in &prog.imports {
        let first = imp.split('.').next().unwrap_or("");
        if matches!(
            first,
            "java" | "std" | "core" | "alloc" | "crate" | "self" | "super"
        ) || local_roots.contains(first)
        {
            continue;
        }
        external.push(imp);
    }
    let async_used = prog.classes.iter().any(|c| c.methods.iter().any(|m| m.is_async));
    if external.is_empty() && !async_used {
        return out;
    }
    let manifest = match find_manifest(src) {
        Some(m) => m,
        None => {
            for imp in &external {
                let krate = imp.split('.').next().unwrap_or("");
                out.push(
                    diag::Diag::error_code(
                        1,
                        "JR5001",
                        format!("`{}` 用到的 crate `{}` 找不到对应的 Cargo.toml", imp, krate),
                    )
                    .with_hint("把源文件放进带 Cargo.toml 的工程目录，或删掉这个 import。"),
                );
            }
            return out;
        }
    };
    let deps = manifest_deps(&manifest);
    for imp in &external {
        let krate = imp.split('.').next().unwrap_or("");
        let normalized = krate.replace('-', "_");
        if !deps.iter().any(|d| *d == normalized) {
            out.push(
                diag::Diag::error_code(
                    1,
                    "JR5001",
                    format!(
                        "`{}` 依赖的 crate `{}` 不在 {} 的 [dependencies] 里",
                        imp,
                        krate,
                        manifest.display()
                    ),
                )
                .with_hint(format!(
                    "在 Cargo.toml 的 [dependencies] 添加 `{} = \"…\"`；jrust 只读取校验，不生成也不改写 Cargo.toml。",
                    krate
                )),
            );
        }
    }
    if async_used && is_under_src(src, &manifest) && !deps.iter().any(|d| d == "tokio") {
        out.push(
            diag::Diag::error_code(
                1,
                "JR5001",
                format!(
                    "async 代码依赖 tokio，但 {} 的 [dependencies] 里没有 tokio",
                    manifest.display()
                ),
            )
            .with_hint(
                "在 Cargo.toml 添加 `tokio = { version = \"1\", features = [\"macros\", \"rt-multi-thread\"] }`。",
            ),
        );
    }
    out
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
    let dep_diags = validate_deps(path, &prog, &HashSet::new());
    if !dep_diags.is_empty() {
        print_diags(path, &dep_diags);
        return false;
    }
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

    // JR-M4：工程内所有 package 的顶层段都是本地模块，其余 import 按外部 crate 校验。
    let local_roots: HashSet<String> = units
        .iter()
        .filter_map(|u| u.prog.package.as_ref())
        .filter_map(|p| p.split('.').next().map(|s| s.to_string()))
        .collect();
    for u in &units {
        let dep_diags = validate_deps(Path::new(&u.src_name), &u.prog, &local_roots);
        if !dep_diags.is_empty() {
            print_diags(Path::new(&u.src_name), &dep_diags);
            ok = false;
        }
    }
    if !ok {
        return false;
    }

    let mut files_out: BTreeMap<PathBuf, String> = BTreeMap::new();
    let mut sources: BTreeMap<Vec<String>, PathBuf> = BTreeMap::new();
    for u in &units {
        let code = match codegen::generate_multi_with_modules(&u.prog, &u.src_name, &local_roots) {
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
    let dep_diags = validate_deps(path, &prog, &HashSet::new());
    if !dep_diags.is_empty() {
        print_diags(path, &dep_diags);
        return false;
    }
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

/// 格式化扫描状态：跨行块注释。
#[derive(Default)]
struct ScanState {
    block_comment: bool,
}

/// 去掉字符串 / 字符 / 注释后的「代码骨架」，只用于数括号和识别 `raw {`。
fn code_only(line: &str, st: &mut ScanState) -> String {
    let chars: Vec<char> = line.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if st.block_comment {
            if c == '*' && i + 1 < chars.len() && chars[i + 1] == '/' {
                st.block_comment = false;
                i += 2;
            } else {
                i += 1;
            }
            continue;
        }
        if c == '/' && i + 1 < chars.len() && chars[i + 1] == '/' {
            break;
        }
        if c == '/' && i + 1 < chars.len() && chars[i + 1] == '*' {
            st.block_comment = true;
            i += 2;
            continue;
        }
        if c == '"' {
            i += 1;
            while i < chars.len() {
                if chars[i] == '\\' {
                    i += 2;
                    continue;
                }
                if chars[i] == '"' {
                    i += 1;
                    break;
                }
                i += 1;
            }
            continue;
        }
        if c == '\'' {
            let mut j = i + 1;
            let mut closed = false;
            while j < chars.len() {
                if chars[j] == '\\' {
                    j += 2;
                    continue;
                }
                if chars[j] == '\'' {
                    closed = true;
                    break;
                }
                j += 1;
            }
            if closed {
                i = j + 1;
                continue;
            }
        }
        out.push(c);
        i += 1;
    }
    out
}

fn brace_net(code: &str) -> isize {
    code.matches('{').count() as isize - code.matches('}').count() as isize
}

/// 若这一行开始了一个跨行的 `raw {`，返回 raw 内部未闭合的花括号数。
fn raw_open_balance(code: &str) -> Option<usize> {
    let chars: Vec<char> = code.chars().collect();
    let mut i = 0;
    while i + 3 <= chars.len() {
        let word = chars[i] == 'r' && chars[i + 1] == 'a' && chars[i + 2] == 'w';
        let boundary = i == 0 || !(chars[i - 1].is_alphanumeric() || chars[i - 1] == '_');
        let after = i + 3 == chars.len() || !(chars[i + 3].is_alphanumeric() || chars[i + 3] == '_');
        if word && boundary && after {
            let mut j = i + 3;
            while j < chars.len() && chars[j].is_whitespace() {
                j += 1;
            }
            if j < chars.len() && chars[j] == '{' {
                let rest: String = chars[j..].iter().collect();
                let net = brace_net(&rest);
                return if net > 0 { Some(net as usize) } else { None };
            }
        }
        i += 1;
    }
    None
}

/// `jrust fmt`：统一 4 空格缩进、去行尾空白、空行最多留一行；`raw` 块内容逐字保留。
fn format_jrust(src: &str) -> String {
    let mut out = String::new();
    let mut depth = 0usize;
    let mut raw: Option<usize> = None;
    let mut blank = false;
    let mut scan = ScanState::default();
    for raw_line in src.replace("\r\n", "\n").lines() {
        let line = raw_line.trim_end();
        if let Some(d) = raw {
            // raw 内容原样保留（只去行尾空白），并在 raw 闭合时归还外层缩进层级。
            let code = code_only(line, &mut scan);
            let net = brace_net(&code);
            let next = if net >= 0 {
                d + net as usize
            } else {
                d.saturating_sub((-net) as usize)
            };
            out.push_str(line);
            out.push('\n');
            if next == 0 {
                raw = None;
                depth = depth.saturating_sub(1);
            } else {
                raw = Some(next);
            }
            blank = false;
            continue;
        }
        if line.trim().is_empty() {
            if !blank && !out.is_empty() {
                out.push('\n');
            }
            blank = true;
            continue;
        }
        blank = false;
        let code = code_only(line, &mut scan);
        let leading_closes = code.trim_start().chars().take_while(|c| *c == '}').count();
        let effective = depth.saturating_sub(leading_closes);
        out.push_str(&"    ".repeat(effective));
        out.push_str(line.trim_start());
        out.push('\n');
        let net = brace_net(&code);
        depth = if net >= 0 {
            depth + net as usize
        } else {
            depth.saturating_sub((-net) as usize)
        };
        raw = raw_open_balance(&code);
    }
    while out.ends_with("\n\n") {
        out.pop();
    }
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

fn fmt_one(path: &Path) -> bool {
    let src = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("无法读取 {}: {}", path.display(), e);
            return false;
        }
    };
    let formatted = format_jrust(&src);
    if formatted == src {
        println!("无需修改 {}", path.display());
        return true;
    }
    if let Err(e) = std::fs::write(path, formatted) {
        eprintln!("无法写入 {}: {}", path.display(), e);
        return false;
    }
    println!("已格式化 {}", path.display());
    true
}

/// 生成的 Rust 行号 → (.jrust 行号, 相对偏移)：取最近的 `// jrust:L<n>` 标记。
fn map_to_jrust(gen: &str, gen_line: usize) -> (usize, usize) {
    let mut jline = 0usize;
    let mut marker_line = 0usize;
    for (i, l) in gen.lines().enumerate() {
        let no = i + 1;
        if no > gen_line {
            break;
        }
        if let Some(rest) = l.trim().strip_prefix("// jrust:L") {
            if let Ok(n) = rest.trim().parse::<usize>() {
                jline = n;
                marker_line = no;
            }
        }
    }
    (jline, gen_line.saturating_sub(marker_line))
}

/// 提取 rustc 的 `error[..]` 消息及其后 `--> path:line:col` 的生成代码行号。
fn scan_rustc_errors(stderr: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut pending: Option<String> = None;
    for raw in stderr.lines() {
        let line = raw.trim();
        if let Some(pos) = line.strip_prefix("--> ") {
            let mut parts = pos.rsplitn(3, ':');
            let _col = parts.next();
            let gen_line = parts.next().and_then(|s| s.trim().parse::<usize>().ok());
            if let (Some(gen_line), Some(msg)) = (gen_line, pending.take()) {
                out.push((gen_line, msg));
            }
        } else if line.starts_with("error[") || line.starts_with("error:") {
            pending = Some(line.to_string());
        }
    }
    out
}

/// `jrust verify <文件>`：转译后交给 rustc 编译（lib 模式，不要求 main），
/// 再把 rustc 的报错映射回 `.jrust` 行（JR-C14 / JR-X2）。
fn verify_one(path: &Path) -> bool {
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
    let code = match codegen::generate(&prog, &path.display().to_string()) {
        Ok(c) => c,
        Err(diags) => {
            print_diags(path, &diags);
            return false;
        }
    };
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let base = std::env::temp_dir().join(format!("jrust-verify-{}-{}", std::process::id(), stamp));
    let rs = base.with_extension("rs");
    if let Err(e) = std::fs::write(&rs, &code) {
        eprintln!("无法写入 {}: {}", rs.display(), e);
        return false;
    }
    let out = std::process::Command::new("rustc")
        .args([
            "--edition",
            "2021",
            "-A",
            "warnings",
            "--crate-type",
            "lib",
            "--emit=metadata",
            "-o",
        ])
        .arg(&base)
        .arg(&rs)
        .output();
    let _ = std::fs::remove_file(&rs);
    let _ = std::fs::remove_file(base.with_extension("rmeta"));
    let out = match out {
        Ok(o) => o,
        Err(e) => {
            eprintln!("无法运行 rustc（{}）；请确认 rustc 在 PATH 里", e);
            return false;
        }
    };
    if out.status.success() {
        println!("验证通过 {}", path.display());
        return true;
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    let errors = scan_rustc_errors(&stderr);
    let mut reported = 0;
    for (gen_line, rest) in errors {
        let (jline, off) = map_to_jrust(&code, gen_line);
        eprintln!("错误: {}:{} - rustc {}", path.display(), jline, rest);
        if off > 0 {
            eprintln!(
                "  提示: 该错误在生成代码第 {} 行，对应原 .jrust 第 {} 行附近（raw 块内第 {} 行）",
                gen_line,
                jline + off,
                off
            );
        }
        if rest.contains("E0277") {
            eprintln!(
                "  提示: 类型不满足 trait 约束（如 Send / Sync）。跨线程共享状态用 @shared（Arc<Mutex<T>>），spawn 只能捕获 Send + 'static 的值。"
            );
        }
        reported += 1;
    }
    if reported == 0 {
        eprintln!("rustc 编译失败（未能定位到 .jrust 行）：\n{}", stderr);
    }
    false
}

/// `jrust verify`（无参数）：跑仓库里的一致性套件（JR-G5）。
fn verify_suite() -> bool {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let scripts = [
        "tests/l1_cases.sh",
        "tests/move_cases.sh",
        "tests/interp_parity.sh",
    ];
    let mut ok = true;
    for s in scripts {
        let p = root.join(s);
        if !p.is_file() {
            eprintln!("找不到一致性套件脚本 {}", p.display());
            ok = false;
            continue;
        }
        println!("== 运行 {} ==", s);
        match std::process::Command::new("bash").arg(&p).status() {
            Ok(st) if st.success() => {}
            Ok(_) => ok = false,
            Err(e) => {
                eprintln!("无法运行 {}: {}", p.display(), e);
                ok = false;
            }
        }
    }
    ok
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
