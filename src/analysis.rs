//! 借用推断（JR-O4 / JR-O7 / JR-O9 / JR-O10 / JR-T5）。
//!
//! 这一层只回答「方法签名怎么生成」，不参与解释执行：
//! - 接收者两态：方法体是否真的改了 `this` 的状态 → `&mut self` / `&self`（O7）；
//! - 参数三态 `Ref` / `MutRef` / `Value`（O4/O5）；
//! - 纯 getter 借出：`return this.f;` 且 `f` 是 `String` → 返回 `&str`（O9/T5）；
//! - `return this` 在这里直接拦住，报 `JR4003`（builder 风格本版不做）。
//!
//! 分析结果进 `Sigs`，`codegen` 按 `(类型名, 方法名)` 查表渲染签名；
//! 解释器是纯值语义，不受影响。

use std::collections::{HashMap, HashSet};

use crate::ast::*;
use crate::diag::Diag;

/// 参数三态。`join` 的偏序：`Ref < MutRef < Value`（owned 天然支持读写）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PMode {
    Ref,
    MutRef,
    Value,
}

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

/// 借用返回的来源：`&self` 或第 `i` 个形参。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Origin {
    SelfRef,
    Param(usize),
}

/// 返回值形态：owned，或借出某个 / 某些来源（→ `-> &T`，多个来源时统一 `'a`）。
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum RetMode {
    Owned,
    Borrow(Vec<Origin>),
}

impl RetMode {
    pub fn is_borrow(&self) -> bool {
        matches!(self, RetMode::Borrow(_))
    }
}

/// 一个方法的签名推断结果。
#[derive(Clone, Debug)]
pub struct Sig {
    pub recv: Recv,
    pub params: Vec<PMode>,
    pub ret: RetMode,
}

impl Sig {
    fn new(n: usize) -> Self {
        Sig {
            recv: Recv::Ref,
            // 偏序的底：没人要求所有权就是只读借用（`Ref < MutRef < Value`）。
            params: vec![PMode::Ref; n],
            ret: RetMode::Owned,
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

    pub fn params(&self, ty: &str, name: &str) -> Option<Vec<PMode>> {
        self.get(ty, name).map(|s| s.params.clone())
    }

    pub fn ret(&self, ty: &str, name: &str) -> RetMode {
        self.get(ty, name).map_or(RetMode::Owned, |s| s.ret.clone())
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

    /// 方法名 → 需要 `&mut` 的实参位置（名字级并集：多写 `mut` 只产生警告）。
    pub fn mutref_positions(&self) -> HashMap<String, HashSet<usize>> {
        let mut out: HashMap<String, HashSet<usize>> = HashMap::new();
        for ((_, name), sig) in &self.map {
            for (i, m) in sig.params.iter().enumerate() {
                if *m == PMode::MutRef {
                    out.entry(name.clone()).or_default().insert(i);
                }
            }
        }
        out
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

    // 不动点：接收者 / 参数三态会与「调用其它方法」「接口签名并集」互相影响。
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

                // 参数三态：自身用法 ∪（实现的）接口方法参数（D9 并集）。
                let generics = generic_names(&c.type_params, &m.type_params);
                let mut modes = infer_param_modes(
                    prog, Some(c), &c.name, &m.params, &generics, &m.body, &sigs,
                );
                for ity in &c.implements {
                    if let Some(name) = ty_name(ity) {
                        if let Some(im) = iface_method_params(&sigs, &name, &m.name) {
                            for (i, t) in im.iter().enumerate() {
                                if let Some(x) = modes.get_mut(i) {
                                    *x = x.join(*t);
                                }
                            }
                        }
                    }
                }
                let sig = sigs.entry(&c.name, &m.name, m.params.len());
                for (i, t) in modes.into_iter().enumerate() {
                    if let Some(cur) = sig.params.get_mut(i) {
                        let n = cur.join(t);
                        if *cur != n {
                            *cur = n;
                            changed = true;
                        }
                    }
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

                // 参数三态：默认方法自身用法 ∪ 所有实现类（D9 并集）。
                let mut modes = match &m.body {
                    Some(body) => {
                        let generics = generic_names(&i.type_params, &m.type_params);
                        let params: Vec<Param> = m
                            .params
                            .iter()
                            .map(|p| Param {
                                ty: p.ty.clone(),
                                name: p.name.clone(),
                            })
                            .collect();
                        infer_param_modes(prog, None, &i.name, &params, &generics, body, &sigs)
                    }
                    // 抽象声明从偏序的底起步，随后与所有实现类求并集。
                    None => vec![PMode::Ref; m.params.len()],
                };
                for c in &prog.classes {
                    if !implements_iface(c, &i.name, prog) {
                        continue;
                    }
                    if let Some(im) = sigs.params(&c.name, &m.name) {
                        for (k, t) in im.iter().enumerate() {
                            if let Some(x) = modes.get_mut(k) {
                                *x = x.join(*t);
                            }
                        }
                    }
                }
                let sig = sigs.entry(&i.name, &m.name, m.params.len());
                for (k, t) in modes.into_iter().enumerate() {
                    if let Some(cur) = sig.params.get_mut(k) {
                        let n = cur.join(t);
                        if *cur != n {
                            *cur = n;
                            changed = true;
                        }
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }

    let ret_diags = infer_ret_modes(prog, &mut sigs);
    diags.extend(ret_diags);
    (sigs, diags)
}

/// 返回值来源推断（B2 / §13.2.2）。
///
/// 借用只可能来自 `&self` 与借用形参，因此不需要完整区域推断：对每个 `return`
/// 表达式求「来源集合」，方法级并集即可。多个来源统一成一个 `'a`；来源不可借用
/// （`&mut` 形参、类型不一致、形参已按值使用）时 `JR3011`。
fn infer_ret_modes(prog: &Program, sigs: &mut Sigs) -> Vec<Diag> {
    // 1) 单调不动点：`return helper(p)` 的来源随被调方法的 ret 增长而增长。
    let limit = prog
        .classes
        .iter()
        .map(|c| c.methods.len())
        .sum::<usize>()
        + prog
            .interfaces
            .iter()
            .map(|i| i.methods.len())
            .sum::<usize>()
        + 4;
    for _ in 0..limit {
        let mut changed = false;
        for c in &prog.classes {
            for m in &c.methods {
                if m.is_static || m.is_async || !m.throws.is_empty() {
                    continue;
                }
                let want = class_ret(c, m, sigs);
                if sigs.ret(&c.name, &m.name) != want {
                    sigs.entry(&c.name, &m.name, m.params.len()).ret = want;
                    changed = true;
                }
            }
        }
        for i in &prog.interfaces {
            for m in &i.methods {
                if m.is_static {
                    continue;
                }
                let want = iface_ret(
                    prog,
                    i,
                    &m.name,
                    &m.params,
                    &m.type_params,
                    m.body.as_deref(),
                    sigs,
                );
                if sigs.ret(&i.name, &m.name) != want {
                    sigs.entry(&i.name, &m.name, m.params.len()).ret = want;
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }

    // 2) 归一化：trait 与所有实现必须逐字一致（所有权优先，整组降级）。
    for i in &prog.interfaces {
        for m in &i.methods {
            if m.is_static {
                continue;
            }
            let mut impls: Vec<&ClassDecl> = Vec::new();
            for c in &prog.classes {
                if implements_iface(c, &i.name, prog)
                    && c.methods.iter().any(|x| x.name == m.name && !x.is_static)
                {
                    impls.push(c);
                }
            }
            let iface_mode = sigs.ret(&i.name, &m.name);
            match iface_mode {
                RetMode::Borrow(uni) => {
                    for c in impls {
                        let want = RetMode::Borrow(uni.clone());
                        if sigs.ret(&c.name, &m.name) != want {
                            sigs.entry(&c.name, &m.name, m.params.len()).ret = want;
                        }
                    }
                }
                RetMode::Owned => {
                    for c in impls {
                        if sigs.ret(&c.name, &m.name).is_borrow() {
                            sigs.entry(&c.name, &m.name, m.params.len()).ret = RetMode::Owned;
                        }
                    }
                }
            }
        }
    }

    // 3) 校验：借出来源必须都是 `&self` / 只读形参，且类型一致。
    let mut diags = Vec::new();
    for c in &prog.classes {
        for m in &c.methods {
            if m.is_static {
                continue;
            }
            if let Some(d) = validate_ret(prog, c, m, sigs) {
                diags.push(d);
                sigs.entry(&c.name, &m.name, m.params.len()).ret = RetMode::Owned;
            }
        }
    }
    diags
}

/// 方法自身的返回来源（不含接口并集）。
fn class_ret(c: &ClassDecl, m: &Method, sigs: &Sigs) -> RetMode {
    // 只有不可变方法才能借出（`&mut self` 的 getter 仍返回 owned）。
    if sigs.recv(&c.name, &m.name) != Recv::Ref {
        return RetMode::Owned;
    }
    let mut set: Vec<Origin> = Vec::new();
    let mut owned_return = false;
    let generics = generic_names(&c.type_params, &m.type_params);
    walk_stmts_all(&m.body, &mut |s| {
        if let Stmt::Return {
            value: Some(e), ..
        } = s
        {
            match origins_of_expr(Some(c), &c.name, &m.params, &generics, e, sigs) {
                Some(v) if !v.is_empty() => {
                    for o in v {
                        if !set.contains(&o) {
                            set.push(o);
                        }
                    }
                }
                _ => owned_return = true,
            }
        }
    });
    if owned_return || set.is_empty() {
        RetMode::Owned
    } else {
        RetMode::Borrow(set)
    }
}

/// 接口方法的返回来源：默认方法自身 ∪ 所有实现；任一实现是 owned → 整组 owned。
fn iface_ret(
    prog: &Program,
    i: &InterfaceDecl,
    name: &str,
    params: &[Param],
    type_params: &[TypeParam],
    body: Option<&[Stmt]>,
    sigs: &Sigs,
) -> RetMode {
    let mut set: Vec<Origin> = Vec::new();
    let mut any = false;
    if let Some(body) = body {
        let mut owned = false;
        let generics = generic_names(type_params, &[]);
        walk_stmts_all(body, &mut |s| {
            if let Stmt::Return {
                value: Some(e), ..
            } = s
            {
                match origins_of_expr(None, &i.name, params, &generics, e, sigs) {
                    Some(v) if !v.is_empty() => {
                        any = true;
                        for o in v {
                            if !set.contains(&o) {
                                set.push(o);
                            }
                        }
                    }
                    _ => owned = true,
                }
            }
        });
        if owned {
            return RetMode::Owned;
        }
    }
    for c in &prog.classes {
        if !implements_iface(c, &i.name, prog)
            || !c.methods.iter().any(|x| x.name == name && !x.is_static)
        {
            continue;
        }
        match sigs.ret(&c.name, name) {
            RetMode::Borrow(srcs) => {
                any = true;
                for o in srcs {
                    if !set.contains(&o) {
                        set.push(o);
                    }
                }
            }
            RetMode::Owned => return RetMode::Owned,
        }
    }
    if any && !set.is_empty() {
        RetMode::Borrow(set)
    } else {
        RetMode::Owned
    }
}

/// `return` 表达式的来源集合；`None` 表示 owned（字面量 / 局部变量 / 临时值）。
fn origins_of_expr(
    class: Option<&ClassDecl>,
    self_name: &str,
    params: &[Param],
    generics: &HashSet<String>,
    e: &Expr,
    sigs: &Sigs,
) -> Option<Vec<Origin>> {
    match unparen(e) {
        Expr::Ident(n) => params
            .iter()
            .position(|p| &p.name == n)
            .and_then(|i| {
                if borrowable(&params[i].ty) && !contains_generic(&params[i].ty, generics) {
                    Some(vec![Origin::Param(i)])
                } else {
                    None
                }
            }),
        Expr::Field(base, f) => {
            if matches!(unparen(base), Expr::This) {
                // 只借出 String 字段（类 / 集合字段仍返回 owned + clone，D4 ③）。
                let cd = class?;
                let ok = cd.fields.iter().any(|fd| {
                    &fd.name == f && fd.ty == Type::String && !is_shared_field(fd)
                });
                if ok {
                    Some(vec![Origin::SelfRef])
                } else {
                    None
                }
            } else {
                origins_of_expr(class, self_name, params, generics, base, sigs)
            }
        }
        Expr::Call { recv, name, args } => {
            let mode = callee_ret(class, self_name, params, recv, name, sigs)?;
            let RetMode::Borrow(srcs) = mode else {
                return None;
            };
            let mut out: Vec<Origin> = Vec::new();
            for o in srcs {
                let sub = match o {
                    Origin::SelfRef => {
                        let r = recv.as_ref()?;
                        origins_of_expr(class, self_name, params, generics, r, sigs)?
                    }
                    Origin::Param(j) => {
                        let a = args.get(j)?;
                        origins_of_expr(class, self_name, params, generics, a, sigs)?
                    }
                };
                for x in sub {
                    if !out.contains(&x) {
                        out.push(x);
                    }
                }
            }
            if out.is_empty() {
                None
            } else {
                Some(out)
            }
        }
        _ => None,
    }
}

/// 解析被调方法的返回模式（只处理能静态确定接收者类型的情形）。
fn callee_ret(
    class: Option<&ClassDecl>,
    self_name: &str,
    params: &[Param],
    recv: &Option<Box<Expr>>,
    name: &str,
    sigs: &Sigs,
) -> Option<RetMode> {
    let ty = match recv {
        None => Some(self_name.to_string()),
        Some(r) => match unparen(r) {
            Expr::This => Some(self_name.to_string()),
            Expr::Ident(n) if !params.iter().any(|p| &p.name == n) => Some(n.clone()),
            _ => static_type_of(class, params, r).and_then(|t| ty_name(&t)),
        },
    };
    if let Some(t) = &ty {
        if let Some(s) = sigs.get(t, name) {
            return Some(s.ret.clone());
        }
    }
    None
}

/// 表达式的声明类型（形参 / `this` / `this.字段` / 形参字段）。
fn static_type_of(class: Option<&ClassDecl>, params: &[Param], e: &Expr) -> Option<Type> {
    match unparen(e) {
        Expr::Ident(n) => params
            .iter()
            .find(|p| &p.name == n)
            .map(|p| p.ty.clone()),
        Expr::This => class.map(|c| Type::Named(c.name.clone())),
        Expr::Field(base, f) => {
            if matches!(unparen(base), Expr::This) {
                class.and_then(|c| {
                    c.fields
                        .iter()
                        .find(|x| &x.name == f)
                        .map(|x| x.ty.clone())
                })
            } else {
                // 形参字段：只有拿到类声明才能继续，先返回 None（调用链来源推断会降级 owned）。
                None
            }
        }
        _ => None,
    }
}

/// 能否安全地借出（Copy 标量 / 泛型 / `T?` / 接口 / 枚举都不借）。
fn borrowable(ty: &Type) -> bool {
    match ty {
        Type::String => true,
        Type::List(_) | Type::Array(_) | Type::Map(..) | Type::Set(_) => true,
        Type::Named(_) => true,
        _ => false,
    }
}

/// 借用返回来源合法性校验；不合法 → `JR3011`。
fn validate_ret(prog: &Program, c: &ClassDecl, m: &Method, sigs: &Sigs) -> Option<Diag> {
    let RetMode::Borrow(srcs) = sigs.ret(&c.name, &m.name) else {
        return None;
    };
    if srcs.is_empty() {
        return None;
    }
    if sigs.recv(&c.name, &m.name) != Recv::Ref {
        return Some(jr3011(m.line, "只有 `&self` 方法才能借出返回值"));
    }
    let modes = sigs.params(&c.name, &m.name).unwrap_or_default();
    let mut kinds: Vec<Type> = Vec::new();
    for o in &srcs {
        match o {
            Origin::SelfRef => kinds.push(Type::String),
            Origin::Param(i) => {
                let Some(p) = m.params.get(*i) else {
                    return None;
                };
                let mode = modes.get(*i).copied().unwrap_or(PMode::Value);
                if mode != PMode::Ref {
                    return Some(jr3011(
                        m.line,
                        format!("形参 `{}` 按值或可变借用使用，无法借出", p.name),
                    ));
                }
                if !borrowable(&p.ty) {
                    return Some(jr3011(
                        m.line,
                        format!("形参 `{}` 的类型 `{:?}` 不支持借用返回", p.name, p.ty),
                    ));
                }
                if !type_is_class(prog, &p.ty) {
                    return Some(jr3011(
                        m.line,
                        format!("形参 `{}` 不是普通类，无法借出", p.name),
                    ));
                }
                kinds.push(p.ty.clone());
            }
        }
    }
    if kinds.windows(2).any(|w| w[0] != w[1]) {
        return Some(jr3011(m.line, "多个返回来源的类型不一致"));
    }
    None
}

/// 普通类（非接口 / 非枚举 / 非泛型）：`Borrow` 返回可以安全落地为 `.clone()`。
fn type_is_class(prog: &Program, ty: &Type) -> bool {
    match ty {
        Type::Named(n) => prog.classes.iter().any(|c| &c.name == n),
        _ => true,
    }
}

fn jr3011(line: usize, why: impl Into<String>) -> Diag {
    Diag::error_code(line, "JR3011", format!("借用返回的来源无法统一：{}", why.into()))
        .with_hint("改成返回 owned（显式 `.clone()` / `.to_string()`），或拆成多个方法；也可以让形参保持只读借用。")
}

/// 接口方法的形参三态（供实现类求并集）。
fn iface_method_params(sigs: &Sigs, iface: &str, name: &str) -> Option<Vec<PMode>> {
    sigs.params(iface, name)
}

/// 泛型类型参数名（类 + 方法）：这类形参一律按 `Value`，避免给 `&T` 引入额外 trait 上界。
fn generic_names(c: &[TypeParam], m: &[TypeParam]) -> HashSet<String> {
    c.iter().chain(m.iter()).map(|p| p.name.clone()).collect()
}

/// 参数三态推断（D1–D3 / §13.4）。
fn infer_param_modes(
    prog: &Program,
    class: Option<&ClassDecl>,
    self_name: &str,
    params: &[Param],
    generics: &HashSet<String>,
    body: &[Stmt],
    sigs: &Sigs,
) -> Vec<PMode> {
    let n = params.len();
    let mut u = PUsage {
        names: params.iter().map(|p| p.name.clone()).collect(),
        types: params.iter().map(|p| p.ty.clone()).collect(),
        value: vec![false; n],
        mutref: vec![false; n],
        prog,
        class,
        self_name,
        sigs,
    };
    // Copy 标量 / 泛型：直接 Value。
    for i in 0..n {
        if u.types[i].is_copy_scalar() || contains_generic(&u.types[i], generics) {
            u.value[i] = true;
        }
    }
    u.stmts(body, false);
    (0..n)
        .map(|i| {
            if u.value[i] {
                PMode::Value
            } else if u.mutref[i] {
                PMode::MutRef
            } else {
                PMode::Ref
            }
        })
        .collect()
}

fn contains_generic(t: &Type, generics: &HashSet<String>) -> bool {
    match t {
        Type::Named(n) => generics.contains(n),
        Type::Generic(n, _) => generics.contains(n),
        Type::List(x) | Type::Array(x) | Type::Set(x) => contains_generic(x, generics),
        Type::Map(k, v) => contains_generic(k, generics) || contains_generic(v, generics),
        Type::Nullable(x) => contains_generic(x, generics),
        _ => false,
    }
}

/// 形参用法收集器：一次扫描累计 `Value` / `MutRef` 两类用法。
struct PUsage<'a> {
    names: Vec<String>,
    types: Vec<Type>,
    value: Vec<bool>,
    mutref: Vec<bool>,
    prog: &'a Program,
    class: Option<&'a ClassDecl>,
    self_name: &'a str,
    sigs: &'a Sigs,
}

impl<'a> PUsage<'a> {
    fn idx(&self, n: &str) -> Option<usize> {
        self.names.iter().position(|x| x == n)
    }

    fn mark(&mut self, i: usize, m: PMode) {
        match m {
            PMode::Value => self.value[i] = true,
            PMode::MutRef => self.mutref[i] = true,
            PMode::Ref => {}
        }
    }

    fn stmts(&mut self, stmts: &[Stmt], in_spawn: bool) {
        for s in stmts {
            self.stmt(s, in_spawn);
        }
    }

    fn stmt(&mut self, s: &Stmt, in_spawn: bool) {
        match s {
            Stmt::Let { init, .. } => self.expr(init, in_spawn),
            Stmt::Assign { target, value, .. } => {
                // `p = ..` 需要 owned 形参（&T 不能整体重新赋值）。
                if let Expr::Ident(n) = unparen(target) {
                    if let Some(i) = self.idx(n) {
                        self.mark(i, PMode::Value);
                    }
                }
                // `p.f = ..` / `p[i] = ..` 改的是形参内部 → MutRef。
                if let Some(root) = place_root(target) {
                    if let Some(i) = self.idx(&root) {
                        if !matches!(unparen(target), Expr::Ident(_)) {
                            self.mark(i, PMode::MutRef);
                        }
                    }
                }
                // 把形参直接存进字段（`this.f = p`）→ 字段是 owned，形参必须 Value。
                if let Expr::Ident(n) = unparen(value) {
                    if is_field_place(target) {
                        if let Some(i) = self.idx(n) {
                            self.mark(i, PMode::Value);
                        }
                    }
                }
                self.expr(target, in_spawn);
                self.expr(value, in_spawn);
            }
            Stmt::Return { value: Some(e), .. } => {
                // B2：`return p` 不再直接判成 owned —— 先留给返回值来源推断去借出；
                // 只有显式 `move p` 才是所有权移交。
                self.expr(e, in_spawn);
            }
            Stmt::Expr { expr, .. } => self.expr(expr, in_spawn),
            Stmt::If {
                cond, then, els, ..
            } => {
                self.expr(cond, in_spawn);
                self.stmts(then, in_spawn);
                if let Some(e) = els {
                    self.stmts(e, in_spawn);
                }
            }
            Stmt::While { cond, body, .. } => {
                self.expr(cond, in_spawn);
                self.stmts(body, in_spawn);
            }
            Stmt::ForEach { iter, body, .. } => {
                if let Expr::Call {
                    recv: Some(b),
                    name,
                    args,
                } = unparen(iter)
                {
                    if name == "mutable" && args.is_empty() {
                        if let Expr::Ident(n) = unparen(b) {
                            if let Some(i) = self.idx(n) {
                                self.mark(i, PMode::MutRef);
                            }
                        }
                    }
                }
                self.expr(iter, in_spawn);
                self.stmts(body, in_spawn);
            }
            Stmt::ForRange {
                start, end, body, ..
            } => {
                self.expr(start, in_spawn);
                self.expr(end, in_spawn);
                self.stmts(body, in_spawn);
            }
            Stmt::Switch {
                subject,
                cases,
                default,
                ..
            } => {
                self.expr(subject, in_spawn);
                for c in cases {
                    if let Some(g) = &c.guard {
                        self.expr(g, in_spawn);
                    }
                    self.stmts(&c.body, in_spawn);
                }
                if let Some(d) = default {
                    self.stmts(d, in_spawn);
                }
            }
            Stmt::Throw { expr, .. } => self.expr(expr, in_spawn),
            Stmt::Sync { lock, body, .. } => {
                self.expr(lock, in_spawn);
                self.stmts(body, in_spawn);
            }
            // `spawn` 闭包 `async move` 按值捕获：被捕获的形参一律 owned。
            Stmt::Spawn { body, .. } => self.stmts(body, true),
            _ => {}
        }
    }

    fn expr(&mut self, e: &Expr, in_spawn: bool) {
        match e {
            Expr::Ident(n) => {
                if in_spawn {
                    if let Some(i) = self.idx(n) {
                        self.mark(i, PMode::Value);
                    }
                }
            }
            Expr::Unary {
                op: UnOp::Move,
                expr,
            } => {
                if let Expr::Ident(n) = unparen(expr) {
                    if let Some(i) = self.idx(n) {
                        self.mark(i, PMode::Value);
                    }
                }
                self.expr(expr, in_spawn);
            }
            Expr::Call { recv, name, args } => {
                if let Some(modes) = self.callee_modes(recv, name) {
                    for (i, a) in args.iter().enumerate() {
                        if let (Some(m), Expr::Ident(n)) = (modes.get(i), unparen(a)) {
                            if let Some(k) = self.idx(n) {
                                self.mark(k, *m);
                            }
                        }
                    }
                }
                if let Some(r) = recv {
                    match unparen(r) {
                        Expr::Ident(n) => {
                            if let Some(i) = self.idx(n) {
                                if self.receiver_mut(&self.types[i].clone(), name) {
                                    self.mark(i, PMode::MutRef);
                                }
                                if in_spawn {
                                    self.mark(i, PMode::Value);
                                }
                            }
                        }
                        _ => {
                            if let Some(root) = place_root(r) {
                                if let Some(i) = self.idx(&root) {
                                    let ok = self
                                        .static_type(r)
                                        .map_or(false, |t| self.receiver_mut(&t, name));
                                    if ok {
                                        self.mark(i, PMode::MutRef);
                                    }
                                    if in_spawn {
                                        self.mark(i, PMode::Value);
                                    }
                                }
                            }
                        }
                    }
                    self.expr(r, in_spawn);
                }
                for a in args {
                    self.expr(a, in_spawn);
                }
            }
            Expr::New { args, .. } => {
                // 构造器形参当前一律 Value：把形参交给构造器也算 Value 用法。
                for a in args {
                    if let Expr::Ident(n) = unparen(a) {
                        if let Some(i) = self.idx(n) {
                            self.mark(i, PMode::Value);
                        }
                    }
                    self.expr(a, in_spawn);
                }
            }
            Expr::Field(r, _) | Expr::Paren(r) | Expr::Await(r) | Expr::Try(r) => {
                self.expr(r, in_spawn)
            }
            Expr::Unary { expr, .. } => self.expr(expr, in_spawn),
            Expr::Index(a, b) | Expr::Binary { lhs: a, rhs: b, .. } => {
                self.expr(a, in_spawn);
                self.expr(b, in_spawn);
            }
            Expr::NewArray { len, .. } => self.expr(len, in_spawn),
            Expr::Closure { body, .. } => self.expr(body, in_spawn),
            _ => {}
        }
    }

    /// 被调用方法各形参的三态（只解析能静态确定的情形）。
    fn callee_modes(&self, recv: &Option<Box<Expr>>, name: &str) -> Option<Vec<PMode>> {
        match recv {
            None => self
                .sigs
                .params(self.self_name, name)
                .or_else(|| self.iface_params(name)),
            Some(r) => match unparen(r) {
                Expr::This => self
                    .sigs
                    .params(self.self_name, name)
                    .or_else(|| self.iface_params(name)),
                Expr::Ident(n) if self.is_type_name(n) => self.sigs.params(n, name),
                _ => self
                    .static_type(r)
                    .and_then(|t| ty_name(&t))
                    .and_then(|n| self.sigs.params(&n, name)),
            },
        }
    }

    fn iface_params(&self, name: &str) -> Option<Vec<PMode>> {
        self.prog
            .interfaces
            .iter()
            .find_map(|i| self.sigs.params(&i.name, name))
    }

    fn is_type_name(&self, n: &str) -> bool {
        self.prog.classes.iter().any(|c| c.name == n)
            || self.prog.interfaces.iter().any(|i| i.name == n)
    }

    /// 类型 `ty` 上调用 `name` 是否需要 `&mut`（集合可变方法 / `&mut self` 方法）。
    fn receiver_mut(&self, ty: &Type, name: &str) -> bool {
        let coll = matches!(
            ty,
            Type::List(_) | Type::Array(_) | Type::Map(..) | Type::Set(_)
        );
        if coll && is_mut_builtin(name) {
            return true;
        }
        match ty {
            Type::Named(n) | Type::Generic(n, _) => self.sigs.recv(n, name) == Recv::Mut,
            Type::Nullable(inner) => self.receiver_mut(inner, name),
            _ => false,
        }
    }

    /// 表达式在 `.jrust` 声明层面的静态类型（形参 / `this` / `this.字段` / 形参字段）。
    fn static_type(&self, e: &Expr) -> Option<Type> {
        match unparen(e) {
            Expr::Ident(n) => self.idx(n).map(|i| self.types[i].clone()),
            Expr::This => self.class.map(|c| Type::Named(c.name.clone())),
            Expr::Field(base, f) => {
                if matches!(unparen(base), Expr::This) {
                    self.class
                        .and_then(|c| c.fields.iter().find(|x| &x.name == f).map(|x| x.ty.clone()))
                } else {
                    let n = ty_name(&self.static_type(base)?)?;
                    self.prog
                        .classes
                        .iter()
                        .find(|c| c.name == n)
                        .and_then(|c| c.fields.iter().find(|x| &x.name == f).map(|x| x.ty.clone()))
                }
            }
            _ => None,
        }
    }
}

fn unparen(e: &Expr) -> &Expr {
    match e {
        Expr::Paren(inner) => unparen(inner),
        _ => e,
    }
}

/// 赋值 / 访问目标的最左根标识符（`p.f[i]` → `p`）。
fn place_root(e: &Expr) -> Option<String> {
    match unparen(e) {
        Expr::Ident(n) => Some(n.clone()),
        Expr::Field(b, _) | Expr::Index(b, _) => place_root(b),
        _ => None,
    }
}

/// 赋值目标是否是字段访问（`x.f = ..` / `this.f = ..`）。
fn is_field_place(e: &Expr) -> bool {
    matches!(unparen(e), Expr::Field(..) | Expr::Index(..))
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
    // 调用会改状态的集合方法 / 本类或接口的 `&mut self` 方法；
    // 把 `this` / `this.f` 交给 `&mut` 形参同样要求 `&mut self`。
    each_stmt_expr(body, &mut |e| {
        if let Expr::Call { recv, name, args } = e {
            if call_needs_mut(prog, class, self_name, recv, name, sigs) {
                hit = true;
            }
            let callee = call_callee_params(prog, class, self_name, recv, name, sigs);
            if let Some(modes) = callee {
                for (i, a) in args.iter().enumerate() {
                    if modes.get(i) == Some(&PMode::MutRef) && this_rooted(a) {
                        hit = true;
                    }
                }
            }
        }
    });
    hit
}

/// 实参是否以 `this` 为根（`this`、`this.f`、`this.f[i]`）。
fn this_rooted(e: &Expr) -> bool {
    match unparen(e) {
        Expr::This => true,
        Expr::Field(b, _) | Expr::Index(b, _) => this_rooted(b),
        _ => false,
    }
}

/// 调用点被调方法的形参三态（只在能静态确定接收者类型时返回）。
fn call_callee_params(
    prog: &Program,
    class: Option<&ClassDecl>,
    self_name: &str,
    recv: &Option<Box<Expr>>,
    name: &str,
    sigs: &Sigs,
) -> Option<Vec<PMode>> {
    let ty = match recv {
        None => Some(self_name.to_string()),
        Some(r) => match unparen(r) {
            Expr::This => Some(self_name.to_string()),
            Expr::Field(base, f) if matches!(unparen(base), Expr::This) => class.and_then(|c| {
                c.fields
                    .iter()
                    .find(|x| &x.name == f)
                    .and_then(|x| ty_name(&x.ty))
            }),
            _ => None,
        },
    }?;
    if let Some(p) = sigs.params(&ty, name) {
        return Some(p);
    }
    prog.interfaces
        .iter()
        .find_map(|i| sigs.params(&i.name, name))
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
