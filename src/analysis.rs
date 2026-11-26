//! 借用推断（JR-O4 / JR-O7 / JR-O9 / JR-O10 / JR-T5）。
//!
//! 这一层只回答「方法签名怎么生成」，不参与解释执行：
//! - 接收者两态：方法体是否真的改了 `this` 的状态 → `&mut self` / `&self`（O7）；
//! - 参数三态 `Ref` / `MutRef` / `Value` 的骨架（本阶段统一 `Value`，B 阶段接入）；
//! - `return this` 在这里直接拦住，报 `JR4003`（builder 风格本版不做）。
//!
//! 分析结果进 `Sigs`，`codegen` 按 `(类型名, 方法名)` 查表渲染签名；
//! 解释器是纯值语义，不受影响。

use std::collections::{HashMap, HashSet};

use crate::ast::*;
use crate::diag::Diag;

/// 参数三态。`join` 的偏序：`Ref < MutRef < Value`（owned 天然支持读写）。
///
/// B 阶段接入参数推断后 `Ref` / `MutRef` 才会真正被构造，当前先留好骨架。
#[allow(dead_code)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PMode {
    Ref,
    MutRef,
    Value,
}

#[allow(dead_code)]
impl PMode {
    /// 合并两处用法：任何一处需要所有权就是 `Value`，否则取更强的借用。
    pub fn join(self, other: PMode) -> PMode {
        if self == PMode::Value || other == PMode::Value {
            PMode::Value
        } else if self == PMode::MutRef || other == PMode::MutRef {
            PMode::MutRef
        } else {
            PMode::Ref
        }
    }

    pub fn is_value(self) -> bool {
        self == PMode::Value
    }
}

/// 接收者两态：`&self` / `&mut self`。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Recv {
    Ref,
    Mut,
}

/// 一个方法的签名推断结果。
#[derive(Clone, Debug)]
pub struct Sig {
    pub recv: Recv,
    /// B 阶段接入参数三态；当前统一 `Value`。
    #[allow(dead_code)]
    pub params: Vec<PMode>,
}

impl Sig {
    fn new(n: usize) -> Self {
        Sig {
            recv: Recv::Ref,
            params: vec![PMode::Value; n],
        }
    }
}

/// `(类型名, 方法名)` → 签名。
#[derive(Default, Clone, Debug)]
pub struct Sigs {
    map: HashMap<(String, String), Sig>,
}

impl Sigs {
    pub fn get(&self, ty: &str, name: &str) -> Option<&Sig> {
        self.map.get(&(ty.to_string(), name.to_string()))
    }

    pub fn recv(&self, ty: &str, name: &str) -> Recv {
        self.get(ty, name).map_or(Recv::Ref, |s| s.recv)
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// 所有按 `&mut self` 生成的方法名：调用点要把接收变量标成 `mut`。
    pub fn mut_names(&self) -> HashSet<String> {
        self.map
            .iter()
            .filter(|(_, s)| s.recv == Recv::Mut)
            .map(|((_, name), _)| name.clone())
            .collect()
    }

    fn entry(&mut self, ty: &str, name: &str, n: usize) -> &mut Sig {
        self.map
            .entry((ty.to_string(), name.to_string()))
            .or_insert_with(|| Sig::new(n))
    }
}

/// 过渡开关：`JRUST_BORROW=legacy` 回到「全 Value + &self」的旧行为（F 阶段删除）。
pub fn legacy_mode() -> bool {
    std::env::var("JRUST_BORROW").map_or(false, |v| v == "legacy")
}

/// 分析整个文件：返回签名表与诊断（目前只有 `JR4003`）。
pub fn analyze(prog: &Program) -> (Sigs, Vec<Diag>) {
    let mut sigs = Sigs::default();
    let mut diags = Vec::new();

    for c in &prog.classes {
        for m in &c.methods {
            if !m.is_static {
                sigs.entry(&c.name, &m.name, m.params.len());
            }
            find_return_this(&m.body, &mut diags);
        }
    }
    for i in &prog.interfaces {
        for m in &i.methods {
            if !m.is_static {
                sigs.entry(&i.name, &m.name, m.params.len());
            }
            if let Some(body) = &m.body {
                find_return_this(body, &mut diags);
            }
        }
    }
    if legacy_mode() {
        return (Sigs::default(), Vec::new());
    }

    // 不动点：接收者可能与「调用其它 `&mut self` 方法」「接口签名取并集」互相影响。
    loop {
        let mut changed = false;
        for c in &prog.classes {
            for m in &c.methods {
                if m.is_static {
                    continue;
                }
                let mut want = body_needs_mut(prog, Some(c), &c.name, &m.body, &sigs);
                // 实现接口的方法必须与 trait 接收者一致（D9）。
                for ity in &c.implements {
                    if let Some(name) = ty_name(ity) {
                        if sigs.get(&name, &m.name).is_some() && sigs.recv(&name, &m.name) == Recv::Mut {
                            want = true;
                        }
                    }
                }
                let target = if want { Recv::Mut } else { Recv::Ref };
                let cur = sigs.entry(&c.name, &m.name, m.params.len()).recv;
                if cur != target {
                    sigs.entry(&c.name, &m.name, m.params.len()).recv = target;
                    changed = true;
                }
            }
        }
        for i in &prog.interfaces {
            for m in &i.methods {
                if m.is_static {
                    continue;
                }
                let mut want = m.body.as_ref().map_or(false, |b| {
                    body_needs_mut(prog, None, &i.name, b, &sigs)
                });
                // 任一实现类需要 `&mut self`，trait 声明也要 `&mut self`（D9 并集）。
                for c in &prog.classes {
                    if !implements_iface(c, &i.name, prog) {
                        continue;
                    }
                    if c.methods
                        .iter()
                        .any(|x| x.name == m.name && !x.is_static)
                        && sigs.recv(&c.name, &m.name) == Recv::Mut
                    {
                        want = true;
                    }
                }
                let target = if want { Recv::Mut } else { Recv::Ref };
                let cur = sigs.entry(&i.name, &m.name, m.params.len()).recv;
                if cur != target {
                    sigs.entry(&i.name, &m.name, m.params.len()).recv = target;
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }

    (sigs, diags)
}

/// 方法体是否必须 `&mut self`。
fn body_needs_mut(
    prog: &Program,
    class: Option<&ClassDecl>,
    self_name: &str,
    body: &[Stmt],
    sigs: &Sigs,
) -> bool {
    let mut hit = false;
    // 直接写 `this.<字段>`：`@shared` 字段靠 `Arc<Mutex<T>>` 加锁，只需要 `&self`。
    walk_stmts(body, &mut |s| {
        if let Stmt::Assign { target, .. } = s {
            if let Some(f) = target_writes_this_field(target) {
                let shared = class.map_or(false, |c| {
                    c.fields
                        .iter()
                        .any(|fd| fd.name == f && is_shared_field(fd))
                });
                if !shared {
                    hit = true;
                }
            }
        }
    });
    // 调用会改状态的集合方法 / 本类或接口的 `&mut self` 方法。
    each_stmt_expr(body, &mut |e| {
        if let Expr::Call { recv, name, .. } = e {
            if call_needs_mut(prog, class, self_name, recv, name, sigs) {
                hit = true;
            }
        }
    });
    hit
}

fn call_needs_mut(
    prog: &Program,
    class: Option<&ClassDecl>,
    self_name: &str,
    recv: &Option<Box<Expr>>,
    name: &str,
    sigs: &Sigs,
) -> bool {
    match recv {
        // 裸调用：实例方法里等价于 `self.name()`。
        None => self_method_mut(prog, self_name, name, sigs),
        Some(r) => match &**r {
            Expr::This => self_method_mut(prog, self_name, name, sigs),
            Expr::Field(base, f) if matches!(**base, Expr::This) => {
                if is_mut_builtin(name) {
                    return true;
                }
                let Some(cd) = class else { return false };
                let Some(ft) = cd.fields.iter().find(|x| &x.name == f).map(|x| &x.ty) else {
                    return false;
                };
                match ty_name(ft) {
                    Some(n) => sigs.recv(&n, name) == Recv::Mut,
                    None => false,
                }
            }
            _ => false,
        },
    }
}

/// 本类（或实现的接口）里名为 `name` 的方法是否需要 `&mut self`。
fn self_method_mut(prog: &Program, self_name: &str, name: &str, sigs: &Sigs) -> bool {
    if sigs.recv(self_name, name) == Recv::Mut {
        return true;
    }
    prog.interfaces
        .iter()
        .filter(|i| sigs.get(&i.name, name).is_some())
        .any(|i| sigs.recv(&i.name, name) == Recv::Mut)
}

/// 集合 / Map 的可变方法：调用它们需要 `&mut self`。
fn is_mut_builtin(name: &str) -> bool {
    matches!(
        name,
        "add" | "push" | "insert" | "remove" | "clear" | "put" | "set" | "pop" | "sort"
            | "extend" | "append" | "dedup" | "retain"
    )
}

fn is_shared_field(f: &Field) -> bool {
    f.attrs.iter().any(|a| a.name == "shared")
}

fn ty_name(t: &Type) -> Option<String> {
    match t {
        Type::Named(n) | Type::Generic(n, _) => Some(n.clone()),
        _ => None,
    }
}

/// 类是否（直接或经接口继承）实现了某个接口。
fn implements_iface(c: &ClassDecl, iface: &str, prog: &Program) -> bool {
    let mut seen = HashSet::new();
    for t in &c.implements {
        iface_closure(t, prog, &mut seen);
    }
    seen.contains(iface)
}

fn iface_closure(t: &Type, prog: &Program, out: &mut HashSet<String>) {
    let Some(name) = ty_name(t) else { return };
    if !out.insert(name.clone()) {
        return;
    }
    if let Some(i) = prog.interfaces.iter().find(|i| i.name == name) {
        for e in &i.extends {
            iface_closure(e, prog, out);
        }
    }
}

/// 赋值目标是否落在 `this.<field>` 上（含 `this.f.x = ..` / `this.f[i] = ..`）。
fn target_writes_this_field(target: &Expr) -> Option<String> {
    match target {
        Expr::Field(recv, name) => match &**recv {
            Expr::This => Some(name.clone()),
            inner => target_writes_this_field(inner),
        },
        Expr::Index(recv, _) => target_writes_this_field(recv),
        _ => None,
    }
}

/// `return this;` → JR4003（builder 风格本版不做）。
fn find_return_this(stmts: &[Stmt], diags: &mut Vec<Diag>) {
    walk_stmts_all(stmts, &mut |s| {
        if let Stmt::Return {
            value: Some(Expr::This),
            line,
        } = s
        {
            diags.push(
                Diag::error_code(
                    *line,
                    "JR4003",
                    "不支持 `return this`（builder 风格返回自身）",
                )
                .with_hint(
                    "`&self` 接收者搬不走自身；改成 `return this.clone();` 返回副本，或改写成静态工厂方法。",
                ),
            );
        }
    });
}

/// 遍历语句，**跳过** `synchronized` / `spawn` 子块（它们的字段访问走 `Arc` 锁或闭包捕获，
/// 不会要求方法自己 `&mut self`）。
fn walk_stmts(stmts: &[Stmt], f: &mut dyn FnMut(&Stmt)) {
    for s in stmts {
        f(s);
        match s {
            Stmt::If { then, els, .. } => {
                walk_stmts(then, f);
                if let Some(e) = els {
                    walk_stmts(e, f);
                }
            }
            Stmt::While { body, .. } | Stmt::ForEach { body, .. } | Stmt::ForRange { body, .. } => {
                walk_stmts(body, f)
            }
            Stmt::Switch { cases, default, .. } => {
                for c in cases {
                    walk_stmts(&c.body, f);
                }
                if let Some(d) = default {
                    walk_stmts(d, f);
                }
            }
            Stmt::Sync { .. } | Stmt::Spawn { .. } => {}
            _ => {}
        }
    }
}

/// 遍历语句（含 `synchronized` / `spawn` 子块），用于必须全量扫描的检查。
fn walk_stmts_all(stmts: &[Stmt], f: &mut dyn FnMut(&Stmt)) {
    for s in stmts {
        f(s);
        match s {
            Stmt::If { then, els, .. } => {
                walk_stmts_all(then, f);
                if let Some(e) = els {
                    walk_stmts_all(e, f);
                }
            }
            Stmt::While { body, .. }
            | Stmt::ForEach { body, .. }
            | Stmt::ForRange { body, .. }
            | Stmt::Sync { body, .. }
            | Stmt::Spawn { body, .. } => walk_stmts_all(body, f),
            Stmt::Switch { cases, default, .. } => {
                for c in cases {
                    walk_stmts_all(&c.body, f);
                }
                if let Some(d) = default {
                    walk_stmts_all(d, f);
                }
            }
            _ => {}
        }
    }
}

fn walk_expr(e: &Expr, f: &mut dyn FnMut(&Expr)) {
    f(e);
    match e {
        Expr::Field(r, _)
        | Expr::Paren(r)
        | Expr::Await(r)
        | Expr::Try(r)
        | Expr::Unary { expr: r, .. } => walk_expr(r, f),
        Expr::Index(a, b) | Expr::Binary { lhs: a, rhs: b, .. } => {
            walk_expr(a, f);
            walk_expr(b, f);
        }
        Expr::Call { recv, args, .. } => {
            if let Some(r) = recv {
                walk_expr(r, f);
            }
            for a in args {
                walk_expr(a, f);
            }
        }
        Expr::New { args, .. } => {
            for a in args {
                walk_expr(a, f);
            }
        }
        Expr::NewArray { len, .. } => walk_expr(len, f),
        Expr::Closure { body, .. } => walk_expr(body, f),
        _ => {}
    }
}

/// 遍历语句里出现的表达式（子块由 `walk_stmts` 负责递归）。
fn each_stmt_expr(stmts: &[Stmt], f: &mut dyn FnMut(&Expr)) {
    walk_stmts(stmts, &mut |s| match s {
        Stmt::Let { init, .. } => walk_expr(init, f),
        Stmt::Assign { target, value, .. } => {
            walk_expr(target, f);
            walk_expr(value, f);
        }
        Stmt::Expr { expr, .. } => walk_expr(expr, f),
        Stmt::Return { value: Some(e), .. } => walk_expr(e, f),
        Stmt::If { cond, .. } => walk_expr(cond, f),
        Stmt::While { cond, .. } => walk_expr(cond, f),
        Stmt::ForEach { iter, .. } => walk_expr(iter, f),
        Stmt::ForRange { start, end, .. } => {
            walk_expr(start, f);
            walk_expr(end, f);
        }
        Stmt::Switch {
            subject, cases, ..
        } => {
            walk_expr(subject, f);
            for c in cases {
                if let Some(g) = &c.guard {
                    walk_expr(g, f);
                }
            }
        }
        Stmt::Throw { expr, .. } => walk_expr(expr, f),
        Stmt::Sync { lock, .. } => walk_expr(lock, f),
        _ => {}
    });
}
