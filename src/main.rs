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
                    let p = Path::new(f);
                    let ok = if p.is_dir() {
                        verify_tree(p)
                    } else {
                        verify_one(p)
                    };
                    if !ok {
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
  jrust verify [文件.jrust 或 目录...]     转译 + rustc/cargo 编译校验；不带参数时跑一致性套件
  jrust fmt <文件或目录>                   格式化 .jrust（统一 4 空格缩进，raw 块原样保留）
  jrust dap                                以 DAP 适配器方式运行（供 IDE 连接）

说明:
  - 输入 Java 风格的 .jrust 源码，输出同名的 .rs 文件（与输入同目录）
  - 生成的 .rs 交给 rustc / cargo 编译；产物保持可读，并带 // jrust:L<行号> 源映射
  - `run` / `debug` 使用内置树遍历解释器，断点与变量都作用在 .jrust 源码行上
  - `verify <目录>` 会按最近的 Cargo.toml 重建最小 manifest 后整 crate 编译；
    不支持 build 脚本 / 本地 path 依赖 / workspace 依赖继承（会直接给出诊断）
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

/// 从目录向上找最近的 `Cargo.lock`（workspace 成员的 lock 在 workspace 根）。
fn find_lockfile(dir: &Path) -> Option<PathBuf> {
    let mut dir = dir.to_path_buf();
    loop {
        let candidate = dir.join("Cargo.lock");
        if candidate.is_file() {
            return Some(candidate);
        }
        if !dir.pop() {
            return None;
        }
    }
}

/// 去掉 ANSI 转义序列（`CARGO_TERM_COLOR=always` 时 cargo/rustc 的输出会带颜色）。
///
/// 子进程已经统一加 `--color never`，这里再兜一层：只要解析的是子进程输出，
/// 就不能假设它一定无色 —— 否则 `--> ` 前缀匹配会静默失配，错误全部退化成
/// 「未能定位到 .jrust 行」。
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c == '\x1b' {
            // CSI：ESC '[' 参数… 终止符（@–~）
            if it.next() == Some('[') {
                for c2 in it.by_ref() {
                    if ('@'..='~').contains(&c2) {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

/// 为 `jrust verify <目录>` 重建一份最小可用 manifest（白名单，不再「抄原文件再剥」）。
///
/// 黑名单式删改永远会漏项，而且漏项报的是 cargo 的原始错误 —— 看上去像用户代码写错：
/// `build = "build.rs"`（临时目录没这个文件）、`[package] default-run`（引用的 bin 已被剥掉）、
/// `[workspace]` + members、crate 内 `path` 依赖（相对路径在临时目录下不存在）。
/// 因此这里只保留 `[package]` 的 name/version/edition 与依赖段，其余（含 `[workspace]`、
/// target 段、features、profile）全部丢弃；确实搬不走的情况给出明确诊断。
fn build_verify_manifest(
    text: &str,
    manifest_dir: &Path,
    dir: &Path,
    plan: &TreePlan,
) -> Result<String, String> {
    // 按 section 切块：块名 + 原始行。
    let mut blocks: Vec<(String, Vec<String>)> = vec![(String::new(), Vec::new())];
    for raw in text.lines() {
        let t = raw.trim();
        if t.starts_with('[') && t.ends_with(']') {
            let name = t.trim_matches(|c| c == '[' || c == ']').trim().to_string();
            blocks.push((name, Vec::new()));
        } else {
            blocks.last_mut().unwrap().1.push(raw.to_string());
        }
    }

    let mut name: Option<String> = None;
    let mut version: Option<String> = None;
    let mut edition: Option<String> = None;
    let mut dep_blocks: Vec<(String, Vec<String>)> = Vec::new();
    // `[[bin]]` 原样保留（重定位到待验目录后），否则多 bin 工程只会验到 crate 根那一个。
    let mut bin_blocks: Vec<(Option<String>, String)> = Vec::new();

    for (section, lines) in &blocks {
        match section.as_str() {
            "package" => {
                for l in lines {
                    let t = l.trim();
                    if t.is_empty() || t.starts_with('#') {
                        continue;
                    }
                    let (k, v) = match t.split_once('=') {
                        Some(kv) => kv,
                        None => continue,
                    };
                    let (key, val) = (k.trim(), v.trim());
                    if key == "build" {
                        return Err(format!(
                            "工程使用了 build 脚本（build = {}）；verify 会把产物放进临时目录，搬不走它，请改用 `jrust build` + cargo build",
                            val
                        ));
                    }
                    // 其余 package 字段（authors / description / default-run / readme…）与编译无关。
                    if !matches!(key, "name" | "version" | "edition") {
                        continue;
                    }
                    // 形如 { workspace = true } 的继承值在临时目录里解析不到，回落到默认值。
                    let inherited = val.starts_with('{');
                    match key {
                        "name" if inherited => {
                            return Err("Cargo.toml 的 package.name 来自 workspace 继承，verify 无法独立重建".into())
                        }
                        "name" => name = Some(val.to_string()),
                        "version" => version = Some(if inherited { "0.0.0".into() } else { val.to_string() }),
                        _ => edition = Some(if inherited { "2021".into() } else { val.to_string() }),
                    }
                }
            }
            // 依赖段：与 JR-M4 校验共用 `dep_section_kind` 判定（含 `[dependencies.foo]`
            // 子表与 `[target.'cfg(..)'.dependencies]`）。`dev-` / `build-` 与编译 bin 无关，丢弃。
            s if dep_section_kind(s).is_some_and(|k| k.kind == DepKind::Normal) => {
                // 搬不走的依赖形态用共享解析器判定，口径与 JR-M4 校验完全一致。
                for d in parse_manifest_deps(&format!("[{}]\n{}", s, lines.join("\n"))) {
                    if let Some(why) = d.unsupported_reason() {
                        return Err(format!(
                            "依赖 `{}` {}，verify 的临时工程里无法重建（请改用 `jrust build` + cargo build）",
                            d.name, why
                        ));
                    }
                }
                // 段名与行原样保留：`[dependencies.foo]` 子表重建成同样的子表，
                // 否则 cargo 会把段里的 version/features 当成 `[package]` 的键。
                dep_blocks.push((section.clone(), lines.clone()));
            }
            "bin" => {
                let mut bin_name: Option<String> = None;
                let mut bin_path: Option<String> = None;
                for l in lines {
                    let t = l.trim();
                    if let Some((k, v)) = t.split_once('=') {
                        let v = v.trim().trim_matches('"').to_string();
                        match k.trim() {
                            "name" => bin_name = Some(v),
                            "path" => bin_path = Some(v),
                            _ => {}
                        }
                    }
                }
                if let Some(p) = bin_path {
                    bin_blocks.push((bin_name, p));
                }
            }
            // `[workspace]` 及一切其它段：丢弃 —— 临时工程是独立 crate，不是 workspace 成员。
            _ => {}
        }
    }

    let name = name.unwrap_or_else(|| "jrust-verify".to_string());
    let mut out = format!(
        "[package]\nname = {}\nversion = {}\nedition = {}\n",
        name,
        version.unwrap_or_else(|| "0.0.0".into()),
        edition.unwrap_or_else(|| "2021".into())
    );
    for (section, lines) in &dep_blocks {
        out.push_str(&format!("\n[{}]\n", section));
        for l in lines {
            if !l.trim().is_empty() {
                out.push_str(l.trim_end());
                out.push('\n');
            }
        }
    }
    // 原 [[bin]] 的 path 相对 manifest 目录；映射到待验目录内、且确实是 jrust 产物的才保留
    // （手写 .rs、待验目录之外的目标都无法在临时工程里重建，跳过）。
    let mut bins: Vec<(String, String)> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    for (name, path) in bin_blocks {
        let rel = match manifest_dir.join(&path).strip_prefix(dir) {
            Ok(r) => r.to_path_buf(),
            Err(_) => {
                skipped.push(path);
                continue;
            }
        };
        if !plan.files.contains_key(&rel) {
            skipped.push(path);
            continue;
        }
        let name = name.unwrap_or_else(|| {
            rel.file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "bin".to_string())
        });
        bins.push((name, rel.to_string_lossy().replace('\\', "/")));
    }
    if !skipped.is_empty() {
        // 静默丢弃会让用户以为「整工程都验过了」，明确提示哪些 bin 没参与。
        eprintln!(
            "提示: 跳过 {} 个 [[bin]]（不在待验目录内，或不是 jrust 产物）：{}",
            skipped.len(),
            skipped.join(", ")
        );
    }
    let root = plan.root_out.to_string_lossy().replace('\\', "/");
    if plan.lib_mode {
        out.push_str(&format!("\n[lib]\npath = \"{}\"\n", root));
    } else {
        if !bins.iter().any(|(_, p)| p == &root) {
            bins.push(("jrust-verify-tree".to_string(), root));
        }
        for (name, path) in bins {
            out.push_str(&format!("\n[[bin]]\nname = \"{}\"\npath = \"{}\"\n", name, path));
        }
    }
    Ok(out)
}

/// `Cargo.toml` 里的一个依赖声明。
#[derive(Debug, Clone)]
struct DepDecl {
    /// crate 名（`-` 归一成 `_`，与 Rust 里 `extern crate` 的名字一致）。
    name: String,
    /// 内联值部分（`foo = "1"` / `foo = { path = ".." }` 里 `=` 之后）；子表形式为空。
    value: String,
    /// 子表形式（`[dependencies.foo]`）下该依赖自身的配置行。
    lines: Vec<String>,
}

impl DepDecl {
    /// 该依赖是否用了本地 path 或 workspace 继承（`verify` 的临时工程两者都搬不走）。
    fn unsupported_reason(&self) -> Option<&'static str> {
        let mut hay = vec![self.value.as_str()];
        hay.extend(self.lines.iter().map(|s| s.as_str()));
        for h in hay {
            if h.contains("workspace = true") || h.contains("workspace=true") {
                return Some("使用 workspace 继承");
            }
            if h.contains("path =") || h.contains("path=") {
                return Some("是本地 path 依赖");
            }
        }
        None
    }
}

/// 段名是不是依赖段，返回依赖类别与子表名（`[dependencies.foo]` 里的 `foo`）。
///
/// **唯一的依赖段判定入口**：`build_verify_manifest`（重建临时 manifest）与
/// `manifest_deps`（JR-M4 校验）都必须走这里，否则会出现「校验说没问题、cargo 说没这个 crate」。
///
/// 认这些写法（`target.` 前缀可叠加在任意一种上）：
/// - `dependencies` / `dev-dependencies` / `build-dependencies`
/// - `dependencies.foo`（子表形式，crate 名在段名里）
fn dep_section_kind(section: &str) -> Option<DepSection> {
    // 去掉 `target.<cfg>.` 前缀：段名形如 `target.'cfg(unix)'.dependencies[.foo]`。
    let body = match section.strip_prefix("target.") {
        Some(rest) => match rest.find('.') {
            Some(i) => &rest[i + 1..],
            None => return None,
        },
        None => section,
    };
    let (kind, rest) = if body == "dependencies" {
        (DepKind::Normal, "")
    } else if body == "dev-dependencies" {
        (DepKind::Dev, "")
    } else if body == "build-dependencies" {
        (DepKind::Build, "")
    } else if let Some(r) = body.strip_prefix("dependencies.") {
        (DepKind::Normal, r)
    } else if let Some(r) = body.strip_prefix("dev-dependencies.") {
        (DepKind::Dev, r)
    } else if let Some(r) = body.strip_prefix("build-dependencies.") {
        (DepKind::Build, r)
    } else {
        return None;
    };
    Some(DepSection {
        kind,
        subtable: rest.trim().trim_matches('"').trim_matches('\'').to_string(),
    })
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum DepKind {
    Normal,
    Dev,
    Build,
}

#[derive(Debug, Clone)]
struct DepSection {
    kind: DepKind,
    /// `[dependencies.foo]` 形式时的 crate 名；普通段为空。
    subtable: String,
}

/// 解析 `Cargo.toml` 里的全部依赖声明（含 `target.*` 段与 `[dependencies.foo]` 子表形式）。
///
/// 这是依赖信息的**唯一解析器**：JR-M4 校验与 `verify <目录>` 的 manifest 重建共用它。
fn parse_manifest_deps(text: &str) -> Vec<DepDecl> {
    let mut out: Vec<DepDecl> = Vec::new();
    let mut section = String::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            section = line.trim_matches(|c| c == '[' || c == ']').trim().to_string();
            // `[dependencies.foo]` / `[target.'cfg(unix)'.dependencies.foo]`：crate 名在段名里。
            if let Some(sec) = dep_section_kind(&section) {
                if !sec.subtable.is_empty() {
                    out.push(DepDecl {
                        name: sec.subtable.replace('-', "_"),
                        value: String::new(),
                        lines: Vec::new(),
                    });
                }
            }
            continue;
        }
        // `[dependencies]` 段内的 `foo = "1"` 形式。
        let Some(sec) = dep_section_kind(&section) else {
            continue;
        };
        if !sec.subtable.is_empty() {
            // 子表段里的行属于该依赖本身（version / features），不是新依赖。
            if let Some(last) = out.last_mut() {
                last.lines.push(line.to_string());
            }
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            let name = k.trim().trim_matches('"').trim_matches('\'');
            if !name.is_empty() {
                out.push(DepDecl {
                    name: name.replace('-', "_"),
                    value: v.trim().to_string(),
                    lines: Vec::new(),
                });
            }
        }
    }
    out
}

/// 读取 `Cargo.toml` 里声明的依赖名（`-` 归一成 `_`，与 Rust crate 名一致）。
fn manifest_deps(manifest: &Path) -> Vec<String> {
    let text = std::fs::read_to_string(manifest).unwrap_or_default();
    parse_manifest_deps(&text)
        .into_iter()
        .map(|d| d.name)
        .collect()
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
        Ok(gen) => {
            print_warnings(path, &gen.warnings);
            let out = path.with_extension("rs");
            if let Err(e) = std::fs::write(&out, gen.code) {
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

/// 把多个文件的声明合并成一份「全工程类型表」（只用于**查询**，不用于输出）。
///
/// 与 `analysis::analyze_all` 的合并口径一致：只拼 interfaces / classes / enums，
/// `package` / `imports` 留空（查询侧不读它们）。
fn merge_programs<'a, I>(programs: I) -> ast::Program
where
    I: IntoIterator<Item = &'a ast::Program>,
{
    let mut merged = ast::Program::default();
    for p in programs {
        merged.interfaces.extend(p.interfaces.iter().cloned());
        merged.classes.extend(p.classes.iter().cloned());
        merged.enums.extend(p.enums.iter().cloned());
    }
    merged
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

/// 多文件模式的生成结果：相对路径 → 产物源码，外加 crate 根产物。
struct TreePlan {
    files: BTreeMap<PathBuf, String>,
    root_out: PathBuf,
    /// 没有含 `main` 的根，按 lib crate 处理（根产物 `lib.rs`）。
    lib_mode: bool,
    /// JR-X3：工程内 raw 块数与内容行数（verify 报告单列，不计入诊断覆盖率）。
    raw_blocks: usize,
    raw_lines: usize,
}

/// 多文件模式构建（JR-S4/M3）：
/// - `package a.b.c;` → `a/b/c.rs`，package 最后一段必须等于文件名；
/// - 含 `main` 的文件是 crate 根，不能声明 package；
/// - 每层父模块补 `pub mod 子模块;`：父模块有源文件就写进它的产物，否则生成 `a/mod.rs`。
fn build_tree(dir: &Path) -> bool {
    let plan = match plan_tree(dir) {
        Some(p) => p,
        None => return false,
    };
    let mut ok = true;
    for (rel, content) in &plan.files {
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

/// 解析目录下全部 `.jrust`、做一次全局分析并生成所有产物，**不落盘**。
/// `build_tree` 与 `verify_tree` 共用；诊断已经打印，返回 `None` 表示有错误。
fn plan_tree(dir: &Path) -> Option<TreePlan> {
    let mut files = Vec::new();
    collect_jrust(dir, &mut files);
    files.sort();
    if files.is_empty() {
        eprintln!("{} 下没有 .jrust 文件", dir.display());
        return None;
    }

    struct Unit {
        prog: ast::Program,
        src_name: String,
        out_rel: PathBuf,
        has_main: bool,
    }

    let mut units: Vec<Unit> = Vec::new();
    // 没有 package 也没有 main 的文件：候选 lib crate 根（最多一个）。
    let mut no_pkg_roots: Vec<usize> = Vec::new();
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
                // 包路径的每一段都会成为 Rust 模块名；`crate` / `self` / `Self` / `super`
                // 无法用 `r#` 挽救，只能报诊断（其余关键字由 codegen 加 `r#`）。
                if let Some(bad) = segs.iter().find(|s| codegen::is_keyword_unrecoverable(s)) {
                    print_diags(
                        f,
                        &[diag::Diag::error_code(
                            1,
                            "JR3030",
                            format!(
                                "package 段 `{}` 是 Rust 关键字且无法用 `r#` 转义，不能作为模块名",
                                bad
                            ),
                        )
                        .with_hint("改一个目录名 / 包名，例如 `crate` → `app`、`super` → `base`。")],
                    );
                    ok = false;
                }
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
            // 无 package 且无 main：在多文件工程里它只能是 lib crate 根。
            // 这里先放行，等收集完所有文件再判断「是否恰好只有一个这样的文件」。
            no_pkg_roots.push(units.len());
        }
        units.push(Unit {
            prog,
            src_name: f.display().to_string(),
            out_rel,
            has_main: main_line.is_some(),
        });
    }
    if !ok {
        return None;
    }

    let roots: Vec<usize> = units
        .iter()
        .enumerate()
        .filter(|(_, u)| u.has_main)
        .map(|(i, _)| i)
        .collect();
    if roots.len() > 1 {
        eprintln!("多文件项目里只能有一个含 main 的 crate 根文件");
        return None;
    }
    // 没有 main：按 lib crate 处理。根产物优先用「无 package 的根文件」的产物，
    // 这样用户能在 lib 根里写 `pub struct`；没有这种文件时才自动生成 `lib.rs`
    // 只放模块声明（与单文件 `verify --crate-type lib` 对齐）。
    if no_pkg_roots.len() > 1 {
        eprintln!(
            "多文件项目里最多只能有一个「没有 package 声明」的 crate 根文件（当前 {} 个）：{}",
            no_pkg_roots.len(),
            no_pkg_roots
                .iter()
                .map(|i| units[*i].src_name.clone())
                .collect::<Vec<_>>()
                .join(", ")
        );
        return None;
    }
    let (root_out, lib_mode) = match roots.first() {
        Some(i) => (units[*i].out_rel.clone(), false),
        None => match no_pkg_roots.first() {
            // 无 main 但有一个无 package 的根文件：它自己就是 lib 根。
            Some(i) => (units[*i].out_rel.clone(), true),
            None => (PathBuf::from("lib.rs"), true),
        },
    };

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
        return None;
    }

    // Stage C：诊断按文件跑（行号归属清楚），签名表则对全工程做一次全局分析，
    // 这样跨文件调用也能拿到对方方法的接收者 / 形参三态 / 返回来源。
    for u in &units {
        let (_sigs, diags) = analysis::analyze(&u.prog);
        if !diags.is_empty() {
            print_diags(Path::new(&u.src_name), &diags);
            ok = false;
        }
    }
    if !ok {
        return None;
    }
    let global_sigs = analysis::analyze_all(units.iter().map(|u| &u.prog));
    // 类型表要看全工程：跨文件类不在当前文件的 `prog` 里，而构造器形参类型 /
    // 方法返回类型 / enum / interface / Clone 判定都要读它（v1.25）。
    let global_types = merge_programs(units.iter().map(|u| &u.prog));

    let mut files_out: BTreeMap<PathBuf, String> = BTreeMap::new();
    let mut sources: BTreeMap<Vec<String>, PathBuf> = BTreeMap::new();
    for u in &units {
        let code = match codegen::generate_multi_with_sigs(
            &u.prog,
            &u.src_name,
            &local_roots,
            global_sigs.clone(),
            &global_types,
        ) {
            Ok(gen) => {
                print_warnings(Path::new(&u.src_name), &gen.warnings);
                gen.code
            }
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
        return None;
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
            // 模块名撞 Rust 关键字时写 `pub mod r#box;`（cargo 仍按 `box.rs` 找文件，实测可行）。
            let line = format!("pub mod {};", codegen::rust_ident(kid));
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

    let (raw_blocks, raw_lines) = units.iter().fold((0usize, 0usize), |(b, l), u| {
        let (ub, ul) = codegen::raw_usage(&u.prog);
        (b + ub, l + ul)
    });

    Some(TreePlan {
        files: files_out,
        root_out,
        lib_mode,
        raw_blocks,
        raw_lines,
    })
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
        Ok(gen) => {
            print_warnings(path, &gen.warnings);
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
        let line = strip_ansi(raw.trim());
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

/// JR-X3：raw 内容原样内联、不参与 jrust 诊断，verify 报告里单列块数与行数。
fn print_raw_usage(blocks: usize, lines: usize) {
    if blocks > 0 {
        println!(
            "  raw: {} 块 / {} 行（原样内联，不计入 jrust 诊断覆盖率）",
            blocks, lines
        );
    }
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
    // 多文件工程里单个文件不是自足编译单元：它的 `import app.Util` 需要同工程的其他
    // 产物。这里给出明确提示，而不是让用户对着 `cannot find module or crate app` 猜。
    if let Some(dir) = path.parent() {
        let mut siblings: Vec<PathBuf> = Vec::new();
        collect_jrust(dir, &mut siblings);
        if siblings.len() > 1 {
            eprintln!(
                "提示: {} 所在目录还有 {} 个其他 .jrust 文件；多文件工程请用 `jrust verify {}`（目录模式），单文件校验看不到同工程的模块。",
                path.display(),
                siblings.len() - 1,
                dir.display()
            );
        }
    }
    let prog = match parser::parse_file(&src) {
        Ok(p) => p,
        Err(diags) => {
            print_diags(path, &diags);
            return false;
        }
    };
    let (raw_blocks, raw_lines) = codegen::raw_usage(&prog);
    let code = match codegen::generate(&prog, &path.display().to_string()) {
        Ok(gen) => {
            print_warnings(path, &gen.warnings);
            gen.code
        }
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
        print_raw_usage(raw_blocks, raw_lines);
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

/// `jrust verify <目录>`：多文件工程（L2）的整 crate 校验。
///
/// 单文件模式用 `--crate-type lib --emit=metadata` 就能查，多文件工程不行：
/// ① `mod` 树要求文件按真实路径排布，② 外部 crate（`serde_json` 等）要求依赖可解析。
/// 因此这里把 `plan_tree` 的产物写进临时目录，按原 `Cargo.toml` 重建一份最小 manifest
/// （依赖是唯一事实来源，JR-M4；有 `Cargo.lock` 就一起复用），再用 `cargo build` 编整个
/// crate —— 与用户手工验收同一条路径。
fn verify_tree(dir: &Path) -> bool {
    let plan = match plan_tree(dir) {
        Some(p) => p,
        None => return false,
    };
    let manifest = match find_manifest(dir.join("x.jrust").as_path()) {
        Some(m) => m,
        None => {
            eprintln!(
                "{} 及其上级目录都没有 Cargo.toml；多文件工程必须能解析依赖（JR-M4）",
                dir.display()
            );
            return false;
        }
    };
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let work = std::env::temp_dir().join(format!("jrust-vtree-{}-{}", std::process::id(), stamp));
    if let Err(e) = std::fs::create_dir_all(&work) {
        eprintln!("无法创建临时目录 {}: {}", work.display(), e);
        return false;
    }
    let cleanup = || {
        let _ = std::fs::remove_dir_all(&work);
    };

    // 产物按相对路径落盘；crate 根由 `[[bin]]` 指向它（`main.rs` 不是必需的名字）。
    for (rel, content) in &plan.files {
        let out = work.join(rel);
        if let Some(d) = out.parent() {
            if let Err(e) = std::fs::create_dir_all(d) {
                eprintln!("无法创建 {}: {}", d.display(), e);
                cleanup();
                return false;
            }
        }
        if let Err(e) = std::fs::write(&out, content) {
            eprintln!("无法写入 {}: {}", out.display(), e);
            cleanup();
            return false;
        }
    }
    let manifest_text = std::fs::read_to_string(&manifest).unwrap_or_default();
    let manifest_dir = manifest.parent().unwrap_or_else(|| Path::new("."));
    // 白名单重建：只保留 package 元信息、依赖段与可达的 [[bin]]/[lib]（见 build_verify_manifest）。
    let bin_manifest = match build_verify_manifest(&manifest_text, manifest_dir, dir, &plan) {
        Ok(m) => m,
        Err(msg) => {
            eprintln!("{}/Cargo.toml: {}", manifest_dir.display(), msg);
            cleanup();
            return false;
        }
    };
    if let Err(e) = std::fs::write(work.join("Cargo.toml"), bin_manifest) {
        eprintln!("无法写入临时 Cargo.toml: {}", e);
        cleanup();
        return false;
    }
    // 依赖解析要和用户一致：有 lockfile 就一起带过去，否则 `serde_json = "1"` 之类
    // 会在临时目录里重新解析，可能与用户实际编译的版本不同。
    //
    // workspace 成员的 lockfile 在**根**目录（成员目录下通常没有），所以要沿目录
    // 向上找第一个存在的 `Cargo.lock`；只查 `manifest.parent()` 会让成员工程拿不到
    // 用户实际用的版本（SPEC 说的「以原 Cargo.lock 为准」在成员场景下就不成立）。
    if let Some(lock) = find_lockfile(manifest.parent().unwrap_or_else(|| Path::new("."))) {
        let _ = std::fs::copy(&lock, work.join("Cargo.lock"));
    }

    let run_cargo = |offline: bool| {
        let mut cmd = std::process::Command::new("cargo");
        // `--color never` 必须有：`CARGO_TERM_COLOR=always` 时 `--> ` 会带 ANSI 前缀，
        // 下面的 scan_rustc_errors_multi 会全部失配，错误退化成「未能定位」。
        cmd.args(["build", "--quiet", "--color", "never"])
            .current_dir(&work);
        if offline {
            cmd.arg("--offline");
        }
        cmd.output()
    };
    let mut out = match run_cargo(true) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("无法运行 cargo（{}）；请确认 cargo 在 PATH 里", e);
            cleanup();
            return false;
        }
    };
    // 依赖不在本地缓存 / registry 索引缺失时 `--offline` 会失败；这类失败与用户代码无关，
    // 自动在线重试一次。判据用具体的解析错误文案，而不是「有没有 error」（编译错误也有）。
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).to_lowercase();
        let resolve_issue = is_resolve_issue(&stderr);
        if resolve_issue {
            eprintln!("提示: 依赖不在本地 cargo 缓存里，已去掉 --offline 重试一次");
            match run_cargo(false) {
                Ok(o) => out = o,
                Err(e) => {
                    eprintln!("无法运行 cargo（{}）；请确认 cargo 在 PATH 里", e);
                    cleanup();
                    return false;
                }
            }
        }
    }
    if out.status.success() {
        println!("验证通过 {}", dir.display());
        print_raw_usage(plan.raw_blocks, plan.raw_lines);
        cleanup();
        return true;
    }
    // rustc 报错里的路径是临时目录下的生成文件；映射回原 .jrust 需要产物文本。
    let stderr = String::from_utf8_lossy(&out.stderr);
    let mut reported = 0;
    // 同一条错误可能在同一行出现多次（不同列位），按 (行, 消息) 去重，避免刷屏。
    let mut seen_errs: HashSet<(String, usize, String)> = HashSet::new();
    for (gen_file, gen_line, msg) in scan_rustc_errors_multi(&stderr) {
        let rel = gen_file
            .strip_prefix(work.to_string_lossy().as_ref())
            .unwrap_or(&gen_file)
            .trim_start_matches('/');
        let code = plan
            .files
            .get(Path::new(rel))
            .cloned()
            .unwrap_or_default();
        let (jline, off) = map_to_jrust(&code, gen_line);
        // 生成文件与原 .jrust 同名同目录（`main.rs` ↔ `main.jrust`），报源码侧路径更好读。
        let jpath = dir.join(rel).with_extension("jrust");
        if !seen_errs.insert((jpath.display().to_string(), jline, msg.clone())) {
            continue;
        }
        // 定位不到源行（生成代码开头没有 `// jrust:L` 标记，如 import 行）时说明清楚，
        // 不要只报一个 `:0`。
        if jline == 0 {
            eprintln!(
                "错误: {} - rustc {}（生成代码第 {} 行，位于文件头部的 import / 属性区，无源行映射）",
                jpath.display(),
                msg,
                gen_line
            );
        } else {
            eprintln!("错误: {}:{} - rustc {}", jpath.display(), jline, msg);
        }
        if off > 0 {
            eprintln!(
                "  提示: 该错误在生成代码第 {} 行，对应原 .jrust 第 {} 行附近（raw 块内第 {} 行）",
                gen_line,
                jline + off,
                off
            );
        }
        if msg.contains("E0277") {
            eprintln!(
                "  提示: 类型不满足 trait 约束（如 Send / Sync）。跨线程共享状态用 @shared（Arc<Mutex<T>>），spawn 只能捕获 Send + 'static 的值。"
            );
        }
        reported += 1;
    }
    if reported == 0 {
        eprintln!("cargo 编译失败（未能定位到 .jrust 行）：\n{}", stderr);
    }
    cleanup();
    false
}

/// 提取 `error[..]` 及其 `--> <文件>:<行>:<列>` 的三元组（多文件版：文件也带出来）。
fn scan_rustc_errors_multi(stderr: &str) -> Vec<(String, usize, String)> {
    let mut out = Vec::new();
    let mut pending: Option<String> = None;
    for raw in stderr.lines() {
        let line = strip_ansi(raw.trim());
        if let Some(pos) = line.strip_prefix("--> ") {
            let mut parts = pos.rsplitn(3, ':');
            let _col = parts.next();
            let gen_line = parts.next().and_then(|s| s.trim().parse::<usize>().ok());
            let file = parts.next().unwrap_or("").to_string();
            if let (Some(gen_line), Some(msg)) = (gen_line, pending.take()) {
                out.push((file, gen_line, msg));
            }
        } else if line.starts_with("warning:") || line.starts_with("note:") {
            // warning / note 的 `-->` 不能配给上一个没定位的 error，否则会张冠李戴。
            pending = None;
        } else if line.starts_with("error[") || line.starts_with("error:") {
            // 汇总行（aborting / could not compile）没有 `-->`，留着会污染下一条 warning 的位置。
            if line.starts_with("error: aborting") || line.starts_with("error: could not compile") {
                pending = None;
            } else {
                pending = Some(line.to_string());
            }
        }
    }
    out
}

/// `--offline` 失败是否属于依赖解析 / registry 索引问题（值得在线重试一次）。
/// 入参须已转小写；带 `error[E…]`（rustc 编译错误）一律不算，避免把编译错误重跑一遍。
fn is_resolve_issue(stderr_lower: &str) -> bool {
    !stderr_lower.contains("error[e")
        && (stderr_lower.contains("offline")
            || stderr_lower.contains("no matching package")
            || stderr_lower.contains("registry index was not found")
            || stderr_lower.contains("failed to select a version")
            || stderr_lower.contains("failed to load source")
            || stderr_lower.contains("failed to get")
            || stderr_lower.contains("failed to download")
            || stderr_lower.contains("unable to update")
            || stderr_lower.contains("network"))
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

/// warning 级诊断只提示，不影响退出码。
fn print_warnings(path: &Path, diags: &[diag::Diag]) {
    for d in diags.iter().filter(|d| d.is_warning()) {
        eprintln!("{}", d.render(&path.display().to_string()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// error 没有 `-->` 时，下一条 warning 的 `-->` 不能被当成它的位置（P2：pending 不清空）。
    #[test]
    fn scan_multi_does_not_pair_warning_location_with_previous_error() {
        let stderr = "\
error: aborting due to 1 previous error
warning: unused variable: `x`
 --> /tmp/work/main.rs:3:9
";
        let hits = scan_rustc_errors_multi(stderr);
        assert!(hits.is_empty(), "不该产生任何定位结果，实际 {:?}", hits);
    }

    #[test]
    fn scan_multi_pairs_error_with_location() {
        let stderr = "\
error[E0599]: no method named `nope`
 --> /tmp/work/main.rs:12:20
";
        let hits = scan_rustc_errors_multi(stderr);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].1, 12);
        assert!(hits[0].2.contains("E0599"));
    }

    #[test]
    fn scan_multi_strips_ansi() {
        let stderr = "\u{1b}[1m\u{1b}[91merror[E0308]\u{1b}[0m: mismatched types\n --> /tmp/a.rs:5:1\n";
        let hits = scan_rustc_errors_multi(stderr);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].1, 5);
    }

    /// 离线重试判据：解析类失败要认出来，rustc 编译错误不能误判。
    #[test]
    fn resolve_issue_classification() {
        assert!(is_resolve_issue("error: failed to get `serde` as a dependency"));
        assert!(is_resolve_issue("no matching package named `x` found"));
        assert!(is_resolve_issue("registry index was not found"));
        assert!(is_resolve_issue("failed to select a version for the requirement"));
        assert!(!is_resolve_issue("error[e0599]: no method named `nope` found"));
        assert!(!is_resolve_issue("error: expected expression, found `let` statement"));
    }
}
