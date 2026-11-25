//! 代码生成：把 jrust 的 AST 翻译成可读的 Rust 源码。
//!
//! 生成产物刻意保持「人能读懂」：带缩进、带 `// jrust:L<行号>` 源映射、
//! 遇到语义陷阱时插一条 `// jrust 提示:` 注释。

use std::collections::{HashMap, HashSet};

use crate::ast::*;
use crate::diag::Diag;

pub fn generate(prog: &Program, src_name: &str) -> Result<String, Vec<Diag>> {
    let mut g = Codegen::new(prog);
    g.run(src_name);
    if g.diags.is_empty() {
        Ok(g.out)
    } else {
        Err(g.diags)
    }
}

struct Env {
    vars: HashMap<String, Type>,
    mutated: HashSet<String>,
    ret: Type,
    /// 当前函数是否声明了 `throws`（决定 `?` 是否合法、`return` 是否包 `Ok(...)`）。
    in_throws: bool,
    /// 当前所在类型名（用于推断 `this.字段` 的类型）。
    self_class: Option<String>,
}

impl Env {
    fn new(ret: Type) -> Self {
        Env {
            vars: HashMap::new(),
            mutated: HashSet::new(),
            ret,
            in_throws: false,
            self_class: None,
        }
    }
}

/// 整个程序共用一个错误枚举：单文件模式下所有 `throws` 方法都返回
/// `Result<T, JrustError>`，跨类 `?` 不需要额外的 `From` 转换。
const ERROR_ENUM: &str = "JrustError";

struct Codegen<'a> {
    prog: &'a Program,
    out: String,
    indent: usize,
    ifaces: HashSet<String>,
    classes: HashMap<String, &'a ClassDecl>,
    enums: HashMap<String, &'a EnumDecl>,
    /// 所有 `throws` 列表里出现过的异常名（去重、保持出现顺序）。
    error_types: Vec<String>,
    diags: Vec<Diag>,
    cur_line: usize,
    in_method: bool,
    /// 可以安全 `#[derive(Clone)]` 的类型名（class / enum）。
    cloneable: HashSet<String>,
    /// 当前正在生成的方法 / 类所处的类型参数（`T` 等），视为可 Clone。
    generic_params: HashSet<String>,
}

impl<'a> Codegen<'a> {
    fn new(prog: &'a Program) -> Self {
        let mut ifaces = HashSet::new();
        for i in &prog.interfaces {
            ifaces.insert(i.name.clone());
        }
        let mut classes = HashMap::new();
        for c in &prog.classes {
            classes.insert(c.name.clone(), c);
        }
        let mut enums = HashMap::new();
        for e in &prog.enums {
            enums.insert(e.name.clone(), e);
        }
        let mut error_types: Vec<String> = Vec::new();
        for c in &prog.classes {
            for m in &c.methods {
                for t in &m.throws {
                    let n = simple_type_name(t);
                    if !error_types.contains(&n) {
                        error_types.push(n);
                    }
                }
            }
        }
        // 先假定所有 class / enum 都可 Clone，再按字段类型反复剔除，直到稳定。
        // 起点为「全 true」可以自然处理自引用（List<Node>）而不会递归爆栈。
        let mut cloneable: HashSet<String> = HashSet::new();
        for c in &prog.classes {
            cloneable.insert(c.name.clone());
        }
        for e in &prog.enums {
            cloneable.insert(e.name.clone());
        }
        loop {
            let snapshot = cloneable.clone();
            let mut changed = false;
            for c in &prog.classes {
                let params: HashSet<String> = c.type_params.iter().map(|p| p.name.clone()).collect();
                let ok = c
                    .fields
                    .iter()
                    .filter(|f| !is_const_field(f))
                    .all(|f| type_cloneable(&f.ty, &snapshot, &params));
                if !ok && cloneable.remove(&c.name) {
                    changed = true;
                }
            }
            for e in &prog.enums {
                let params: HashSet<String> = e.type_params.iter().map(|p| p.name.clone()).collect();
                let ok = e
                    .variants
                    .iter()
                    .all(|v| v.params.iter().all(|p| type_cloneable(&p.ty, &snapshot, &params)));
                if !ok && cloneable.remove(&e.name) {
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        Codegen {
            prog,
            out: String::new(),
            indent: 0,
            ifaces,
            classes,
            enums,
            error_types,
            diags: Vec::new(),
            cur_line: 0,
            in_method: false,
            cloneable,
            generic_params: HashSet::new(),
        }
    }

    // ---------- 输出辅助 ----------

    fn emit(&mut self, s: &str) {
        for _ in 0..self.indent {
            self.out.push_str("    ");
        }
        self.out.push_str(s);
        self.out.push('\n');
    }

    fn blank(&mut self) {
        if !self.out.ends_with("\n\n") && !self.out.is_empty() {
            self.out.push('\n');
        }
    }

    fn marker(&mut self, line: usize) {
        self.emit(&format!("// jrust:L{}", line));
    }

    fn err(&mut self, msg: impl Into<String>, hint: impl Into<String>) {
        let line = self.cur_line;
        self.diags.push(Diag::error(line, msg).with_hint(hint));
    }

    fn err_code(&mut self, code: &'static str, msg: impl Into<String>, hint: impl Into<String>) {
        let line = self.cur_line;
        self.diags
            .push(Diag::error_code(line, code, msg).with_hint(hint));
    }

    /// `impl` 头部里的 trait 引用：接口**不要**包 `Box<dyn ...>`。
    fn trait_ref_type(&self, ty: &Type) -> String {
        match ty {
            Type::Named(n) => n.clone(),
            Type::Generic(n, args) => {
                let inner: Vec<String> = args.iter().map(|a| self.rust_type(a)).collect();
                format!("{}<{}>", n, inner.join(", "))
            }
            other => self.rust_type(other),
        }
    }

    /// 这个类型能否按值复制（`.clone()`）。
    fn is_cloneable_type(&self, ty: &Type) -> bool {
        type_cloneable(ty, &self.cloneable, &self.generic_params)
    }

    /// 表达式的值是字符串：字面量、静态类型 `String`，或含字符串操作数的 `+` 链。
    fn expr_is_stringy(&self, e: &Expr, env: &Env) -> bool {
        match strip_parens_expr(e) {
            Expr::Str(_) => true,
            Expr::Binary {
                op: BinOp::Add,
                lhs,
                rhs,
            } => self.expr_is_stringy(lhs, env) || self.expr_is_stringy(rhs, env),
            other => matches!(self.infer_type(other, env), Some(Type::String)),
        }
    }

    /// 把字符串 `+` 链摊平：括号透明，子链递归展开，其余表达式各成一段。
    fn flatten_concat<'e>(&self, e: &'e Expr, env: &Env, out: &mut Vec<&'e Expr>) {
        match strip_parens_expr(e) {
            Expr::Binary {
                op: BinOp::Add,
                lhs,
                rhs,
            } if self.expr_is_stringy(lhs, env) || self.expr_is_stringy(rhs, env) => {
                self.flatten_concat(lhs, env, out);
                self.flatten_concat(rhs, env, out);
            }
            other => out.push(other),
        }
    }

    /// 字符串 `+` → `format!("…{}…", args)`：字面量折进格式串，其余走 `{}`。
    fn gen_concat(&mut self, roots: &[&Expr], env: &mut Env) -> String {
        let mut parts = Vec::new();
        for r in roots {
            self.flatten_concat(r, env, &mut parts);
        }
        let mut fmt = String::new();
        let mut args = Vec::new();
        for p in parts {
            match p {
                Expr::Str(s) => {
                    for ch in s.chars() {
                        match ch {
                            '{' => fmt.push_str("{{"),
                            '}' => fmt.push_str("}}"),
                            _ => fmt.push(ch),
                        }
                    }
                }
                other => {
                    fmt.push_str("{}");
                    args.push(self.gen_expr(other, env, None));
                }
            }
        }
        if args.is_empty() {
            return format!("{:?}.to_string()", fmt);
        }
        format!("format!(\"{}\", {})", fmt, args.join(", "))
    }

    /// `new T[n]` 的默认元素值。元素类型已在 parser 限定为标量 / String。
    fn array_default(&self, elem: &Type) -> String {
        match elem {
            Type::Double | Type::Float => "0.0".to_string(),
            Type::Boolean => "false".to_string(),
            Type::Char => "'\\0'".to_string(),
            Type::String => "String::new()".to_string(),
            Type::RustPrim(n) if n == "bool" => "false".to_string(),
            Type::RustPrim(n) if n.starts_with('f') => "0.0".to_string(),
            _ => "0".to_string(),
        }
    }

    /// 赋值目标 / 声明处的静态类型（用于判断是否要 clone）。
    fn target_type(&self, expr: &Expr, env: &Env) -> Option<Type> {
        match expr {
            Expr::Ident(n) => env.vars.get(n).cloned(),
            Expr::Index(base, _) => match self.infer_type(base, env) {
                Some(Type::Array(inner)) => Some(*inner),
                Some(Type::List(inner)) => Some(*inner),
                _ => None,
            },
            Expr::Field(recv, f) => {
                let base = match &**recv {
                    Expr::This => env.self_class.clone().map(Type::Named),
                    other => self.infer_type(other, env),
                };
                match base {
                    Some(Type::Named(c)) => self.classes.get(&c).and_then(|cd| {
                        instance_fields(cd)
                            .find(|x| &x.name == f)
                            .map(|x| x.ty.clone())
                    }),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// 生成「赋值源」：可空类型补 `Some(...)`，非可空类型从 `T?` 解包，
    /// 并在需要时按值复制（`copy == true`）。
    fn gen_assigned_value(
        &mut self,
        init: &Expr,
        ty: &Type,
        env: &mut Env,
        copy: bool,
        hint_target: Option<&str>,
    ) -> String {
        let core = strip_parens_expr(init);
        match ty {
            Type::Nullable(inner) => {
                if matches!(core, Expr::Null) {
                    return self.gen_expr(init, env, Some(ty));
                }
                if self.expr_is_nullable(core, env) {
                    let s = self.gen_expr(init, env, Some(ty));
                    return if copy {
                        self.clone_if_needed(s, ty, core, hint_target)
                    } else {
                        s
                    };
                }
                let s = self.gen_expr(init, env, Some(inner));
                let s = if copy {
                    self.clone_if_needed(s, inner, core, hint_target)
                } else {
                    s
                };
                format!("Some({})", s)
            }
            Type::Void => self.gen_expr(init, env, Some(ty)),
            other => {
                if self.expr_is_nullable(core, env) {
                    // 只允许把 T? 解包给非可空 T：None 时会 panic，等价于 Java 的 NPE。
                    let base = self.gen_expr(init, env, None);
                    self.emit(
                        "// jrust 提示: 从可能为空的 `T?` 解包，None 会 panic；先确认判过空（`if (x != null)`）。",
                    );
                    return if copy && is_holder_expr(core) && self.is_cloneable_type(other) {
                        format!("{}.clone().unwrap()", base)
                    } else {
                        format!("{}.unwrap()", base)
                    };
                }
                let s = self.gen_expr(init, env, Some(other));
                if copy {
                    self.clone_if_needed(s, other, core, hint_target)
                } else {
                    s
                }
            }
        }
    }

    /// 变量 / 字段是「持有者」：按值语义需要补 `.clone()`。
    fn clone_if_needed(
        &mut self,
        s: String,
        ty: &Type,
        core: &Expr,
        hint_target: Option<&str>,
    ) -> String {
        if !is_holder_expr(core) || ty.is_copy_scalar() || matches!(ty, Type::Void) {
            return s;
        }
        if self.is_cloneable_type(ty) {
            if let (Some(target), Expr::Ident(n)) = (hint_target, core) {
                self.emit(&format!(
                    "// jrust 提示: 普通赋值按值复制——`{}` 与 `{}` 各持一份；只想转移所有权请写 `move {}`。",
                    n, target, n
                ));
            }
            format!("{}.clone()", s)
        } else {
            self.err_code(
                "JR4002",
                format!("`{}` 类型不可 Clone，无法按值复制", self.rust_type(ty)),
                "它含有 trait 对象等不可克隆字段；改成可 Clone 的数据结构，或显式写 `move` 转移所有权。",
            );
            s
        }
    }

    fn expr_is_nullable(&self, e: &Expr, env: &Env) -> bool {
        matches!(self.infer_type(e, env), Some(Type::Nullable(_)))
    }

    /// 表达式是不是 `this` 的字段：从 `&self` 方法返回字段时必须 `.clone()`。
    fn is_self_field(&self, e: &Expr, _env: &Env) -> bool {
        matches!(strip_parens_expr(e), Expr::Field(..))
    }

    /// 方法 / 构造器的形参类型，用于在调用点处理 `T?` 的 `Some` 包装。
    fn call_param_types(
        &self,
        recv: &Option<Box<Expr>>,
        name: &str,
        env: &Env,
    ) -> Option<Vec<Type>> {
        if let Some(Expr::Ident(en)) = recv.as_deref() {
            if let Some(ed) = self.enums.get(en) {
                if let Some(v) = ed.variants.iter().find(|v| &v.name == name) {
                    return Some(v.params.iter().map(|p| p.ty.clone()).collect());
                }
            }
            if let Some(cd) = self.classes.get(en) {
                if let Some(m) = cd.methods.iter().find(|m| &m.name == name && m.is_static) {
                    return Some(m.params.iter().map(|p| p.ty.clone()).collect());
                }
            }
        }
        if let Some(r) = recv.as_deref() {
            if let Some((c, map)) = self.infer_type(r, env).and_then(|t| self.type_binding(&t)) {
                if let Some(cd) = self.classes.get(&c) {
                    if let Some(m) = cd.methods.iter().find(|m| m.name == name && !m.is_static) {
                        return Some(
                            m.params
                                .iter()
                                .map(|p| subst_type(&p.ty, &map))
                                .collect(),
                        );
                    }
                }
                if let Some(iface) = self.prog.interfaces.iter().find(|i| i.name == c) {
                    if let Some(m) = iface.methods.iter().find(|m| m.name == name) {
                        return Some(m.params.iter().map(|p| subst_type(&p.ty, &map)).collect());
                    }
                }
            }
        } else if let Some(c) = &env.self_class {
            if let Some(cd) = self.classes.get(c) {
                if let Some(m) = cd.methods.iter().find(|m| m.name == name && m.is_static) {
                    return Some(m.params.iter().map(|p| p.ty.clone()).collect());
                }
            }
        }
        None
    }

    /// 生成一个实参；形参是 `T?` 时把非空值包成 `Some(...)`。
    fn gen_arg(&mut self, a: &Expr, pt: &Type, env: &mut Env) -> String {
        match pt {
            Type::Nullable(inner) => {
                if matches!(strip_parens_expr(a), Expr::Null) || self.expr_is_nullable(a, env) {
                    self.gen_expr(a, env, Some(pt))
                } else {
                    format!("Some({})", self.gen_expr(a, env, Some(inner)))
                }
            }
            _ => self.gen_expr(a, env, Some(pt)),
        }
    }

    fn rust_type(&self, t: &Type) -> String {
        match t {
            Type::Int => "i32".to_string(),
            Type::Long => "i64".to_string(),
            Type::Double => "f64".to_string(),
            Type::Float => "f32".to_string(),
            Type::Boolean => "bool".to_string(),
            Type::Char => "char".to_string(),
            Type::Void => "()".to_string(),
            Type::String => "String".to_string(),
            Type::RustPrim(name) => name.clone(),
            Type::List(inner) => format!("Vec<{}>", self.rust_type(inner)),
            Type::Map(k, v) => format!(
                "HashMap<{}, {}>",
                self.rust_type(k),
                self.rust_type(v)
            ),
            Type::Array(inner) => format!("Vec<{}>", self.rust_type(inner)),
            Type::Named(n) => {
                if self.ifaces.contains(n) {
                    format!("Box<dyn {}>", n)
                } else {
                    n.clone()
                }
            }
            Type::Generic(n, args) => {
                let inner: Vec<String> = args.iter().map(|a| self.rust_type(a)).collect();
                if self.ifaces.contains(n) {
                    format!("Box<dyn {}<{}>>", n, inner.join(", "))
                } else {
                    format!("{}<{}>", n, inner.join(", "))
                }
            }
            Type::Nullable(inner) => format!("Option<{}>", self.rust_type(inner)),
        }
    }

    // ---------- 顶层 ----------

    fn run(&mut self, src_name: &str) {
        self.emit(&format!(
            "// 由 jrust 从 {} 生成。请对照 .jrust 源码阅读，不要直接改这个文件。",
            src_name
        ));
        self.emit("// 每段代码前的 `// jrust:L<行号>` 指向 .jrust 源码行。");
        self.emit("#![allow(dead_code, unused_variables, unused_mut, unused_imports)]");
        // `java.*` 的 import 只需让 jrust 认识类型名，不生成代码；
        // 其余路径当作 Rust 的 use 原样输出（圆点转 `::`），例如 import std.vec.Vec;
        let imports = self.prog.imports.clone();
        let mut seen = std::collections::HashSet::new();
        for imp in &imports {
            if imp.starts_with("java.") || !seen.insert(imp.clone()) {
                continue;
            }
            self.emit(&format!("use {};", imp.replace('.', "::")));
        }
        // Map 统一映射到 std::collections::HashMap；未用到时由 allow(unused_imports) 兜住。
        if !seen.contains("std.collections.HashMap") {
            self.emit("use std::collections::HashMap;");
        }
        self.blank();

        let prog = self.prog;
        for e in &prog.enums {
            self.gen_enum(e);
        }
        if !self.error_types.is_empty() {
            self.gen_error_enum();
        }
        for iface in &prog.interfaces {
            self.gen_trait(iface);
        }
        for class in &prog.classes {
            let has_main = class.methods.iter().any(|m| m.is_static && m.name == "main");
            if !has_main {
                self.gen_class(class);
            }
        }
        for class in &prog.classes {
            if let Some(main) = class.methods.iter().find(|m| m.is_static && m.name == "main") {
                self.gen_main_class(class, main);
            }
        }
    }

    fn gen_trait(&mut self, iface: &InterfaceDecl) {
        self.marker(iface.line);
        self.emit(&format!(
            "pub trait {}{} {{",
            iface.name,
            type_param_decl(&iface.type_params)
        ));
        self.indent += 1;
        for m in &iface.methods {
            self.marker(m.line);
            let mut ps = vec!["&self".to_string()];
            for p in &m.params {
                ps.push(format!("{}: {}", p.name, self.rust_type(&p.ty)));
            }
            let ret = if m.ret == Type::Void {
                String::new()
            } else {
                format!(" -> {}", self.rust_type(&m.ret))
            };
            self.emit(&format!(
                "fn {}{}({}){};",
                m.name,
                impl_param_decl(&m.type_params),
                ps.join(", "),
                ret
            ));
        }
        self.indent -= 1;
        self.emit("}");
        self.blank();
    }

    fn gen_enum(&mut self, e: &EnumDecl) {
        self.marker(e.line);
        if self.cloneable.contains(&e.name) {
            self.emit("#[derive(Debug, Clone)]");
        } else {
            self.emit("#[derive(Debug)]");
        }
        self.emit(&format!(
            "pub enum {}{} {{",
            e.name,
            type_param_decl(&e.type_params)
        ));
        self.indent += 1;
        for v in &e.variants {
            self.marker(v.line);
            if v.params.is_empty() {
                self.emit(&format!("{},", v.name));
            } else {
                let tys: Vec<String> = v.params.iter().map(|p| self.rust_type(&p.ty)).collect();
                self.emit(&format!("{}({}),", v.name, tys.join(", ")));
            }
        }
        self.indent -= 1;
        self.emit("}");
        self.blank();
    }

    /// 生成程序级错误枚举 + `Display` / `Error` / 常见标准错误的 `From`。
    fn gen_error_enum(&mut self) {
        let types = self.error_types.clone();
        self.emit("#[derive(Debug)]");
        self.emit(&format!("pub enum {} {{", ERROR_ENUM));
        self.indent += 1;
        for name in &types {
            self.emit(&format!("{}(String),", name));
        }
        self.indent -= 1;
        self.emit("}");
        self.blank();

        self.emit(&format!("impl std::fmt::Display for {} {{", ERROR_ENUM));
        self.indent += 1;
        self.emit("fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {");
        self.indent += 1;
        self.emit("match self {");
        self.indent += 1;
        for name in &types {
            self.emit(&format!(
                "{}::{}(m) => write!(f, \"{}: {{}}\", m),",
                ERROR_ENUM, name, name
            ));
        }
        self.indent -= 1;
        self.emit("}");
        self.indent -= 1;
        self.emit("}");
        self.indent -= 1;
        self.emit("}");
        self.emit(&format!("impl std::error::Error for {} {{}}", ERROR_ENUM));
        self.blank();

        for name in types {
            let Some(src) = std_error_source(&name) else {
                continue;
            };
            self.emit(&format!("impl From<{}> for {} {{", src, ERROR_ENUM));
            self.indent += 1;
            self.emit(&format!(
                "fn from(e: {}) -> Self {{ {}::{}(e.to_string()) }}",
                src, ERROR_ENUM, name
            ));
            self.indent -= 1;
            self.emit("}");
            self.blank();
        }
    }

    /// `static final` 字段 → 模块级 `pub const`（JR-D8）。
    fn gen_consts(&mut self, class: &ClassDecl) {
        let consts: Vec<&Field> = class.fields.iter().filter(|f| is_const_field(f)).collect();
        for f in &consts {
            self.marker(f.line);
            self.cur_line = f.line;
            let vis = if f.vis == Vis::Public { "pub " } else { "" };
            let ty = if f.ty == Type::String {
                "&str".to_string()
            } else {
                self.rust_type(&f.ty)
            };
            let val = match (&f.init, &f.ty) {
                (Some(Expr::Str(s)), Type::String) => format!("{:?}", s),
                (Some(init), _) => {
                    let mut env = Env::new(Type::Void);
                    self.gen_expr(init, &mut env, Some(&f.ty))
                }
                (None, _) => String::new(),
            };
            self.emit(&format!("{}const {}: {} = {};", vis, f.name, ty, val));
        }
        if !consts.is_empty() {
            self.blank();
        }
    }

    fn gen_class(&mut self, class: &ClassDecl) {
        let tparams = type_param_decl(&class.type_params);
        let phantom = phantom_params(class);
        let prev_generic = std::mem::replace(
            &mut self.generic_params,
            class.type_params.iter().map(|p| p.name.clone()).collect(),
        );
        self.gen_consts(class);
        self.marker(class.line);
        if self.cloneable.contains(&class.name) {
            self.emit("#[derive(Clone)]");
        }
        if instance_fields(class).next().is_none() && phantom.is_empty() {
            self.emit(&format!("pub struct {}{};", class.name, tparams));
        } else {
            self.emit(&format!("pub struct {}{} {{", class.name, tparams));
            self.indent += 1;
            for f in instance_fields(class) {
                self.marker(f.line);
                let vis = if f.vis == Vis::Public { "pub " } else { "" };
                self.emit(&format!("{}{}: {},", vis, f.name, self.rust_type(&f.ty)));
            }
            if !phantom.is_empty() {
                self.emit(&format!(
                    "_marker: std::marker::PhantomData<{}>,",
                    phantom.join(", ")
                ));
            }
            self.indent -= 1;
            self.emit("}");
        }
        self.blank();

        let impl_params = impl_param_decl(&class.type_params);
        let prog = self.prog;
        let mut iface_methods: HashSet<String> = HashSet::new();
        for ity in &class.implements {
            if let Some(iface) = prog.interfaces.iter().find(|x| x.name == type_name(ity)) {
                for m in &iface.methods {
                    iface_methods.insert(m.name.clone());
                }
            }
        }

        // 实现接口的方法 → impl Trait for Type
        for ity in &class.implements {
            let Some(iface) = prog.interfaces.iter().find(|x| x.name == type_name(ity)) else {
                continue;
            };
            let names: HashSet<&str> = iface.methods.iter().map(|m| m.name.as_str()).collect();
            let methods: Vec<&Method> = class
                .methods
                .iter()
                .filter(|m| !m.is_static && names.contains(m.name.as_str()))
                .collect();
            if methods.is_empty() {
                continue;
            }
            self.emit(&format!(
                "impl{} {} for {}{} {{",
                impl_params,
                self.trait_ref_type(ity),
                class.name,
                tparams
            ));
            self.indent += 1;
            for m in methods {
                self.gen_method(m, true, &class.name);
            }
            self.indent -= 1;
            self.emit("}");
            self.blank();
        }

        // 构造器与自有方法 → impl Type
        let inherent: Vec<&Method> = class
            .methods
            .iter()
            .filter(|m| m.is_static || !iface_methods.contains(&m.name))
            .collect();
        if class.ctor.is_some() || !inherent.is_empty() {
            self.emit(&format!(
                "impl{} {}{} {{",
                impl_params, class.name, tparams
            ));
            self.indent += 1;
            if let Some(ctor) = &class.ctor {
                self.gen_ctor(class, ctor);
            }
            for m in inherent {
                self.gen_method(m, false, &class.name);
            }
            self.indent -= 1;
            self.emit("}");
            self.blank();
        }
        self.generic_params = prev_generic;
    }

    fn gen_main_class(&mut self, class: &ClassDecl, main: &Method) {
        if instance_fields(class).next().is_some() || class.methods.len() > 1 {
            self.cur_line = class.line;
            self.err(
                "含 main 的类只能有 main 方法",
                "jrust MVP 把 main 当作程序入口；其它字段和方法请放进独立的类（static final 常量除外）。",
            );
        }
        self.gen_consts(class);
        self.marker(main.line);
        let throws = !main.throws.is_empty();
        if throws {
            self.emit(&format!("fn main() -> Result<(), {}> {{", ERROR_ENUM));
        } else {
            self.emit("fn main() {");
        }
        self.indent += 1;
        if stmts_use_ident(&main.body, "args") {
            self.emit("let args: Vec<String> = std::env::args().skip(1).collect();");
        }
        let mut env = Env::new(Type::Void);
        for p in &main.params {
            env.vars.insert(p.name.clone(), p.ty.clone());
        }
        env.mutated = collect_mutated(&main.body);
        env.in_throws = throws;
        env.self_class = Some(class.name.clone());
        let prev = self.in_method;
        self.in_method = false;
        self.gen_stmts(&main.body, &mut env);
        self.in_method = prev;
        if throws {
            self.emit("Ok(())");
        }
        self.indent -= 1;
        self.emit("}");
        self.blank();
    }

    fn gen_ctor(&mut self, class: &ClassDecl, ctor: &Constructor) {
        let mut ps = Vec::new();
        for p in &ctor.params {
            ps.push(format!("{}: {}", p.name, self.rust_type(&p.ty)));
        }
        let vis = if ctor.vis == Vis::Public { "pub " } else { "" };
        self.marker(ctor.line);
        self.emit(&format!("{}fn new({}) -> Self {{", vis, ps.join(", ")));
        self.indent += 1;
        let mut env = Env::new(Type::Void);
        for p in &ctor.params {
            env.vars.insert(p.name.clone(), p.ty.clone());
        }
        let mut fields: Vec<String> = Vec::new();
        for st in &ctor.body {
            match st {
                Stmt::Assign {
                    target: Expr::Field(recv, field),
                    value,
                    ..
                } if matches!(**recv, Expr::This) => {
                    if class.fields.iter().any(|f| &f.name == field && is_const_field(f)) {
                        self.cur_line = ctor.line;
                        self.err(
                            format!("不能给静态常量 `{}` 赋值", field),
                            "static final 转发为 `const`，编译期固定；构造器里只给实例字段赋值。",
                        );
                        continue;
                    }
                    let field_ty = instance_fields(class)
                        .find(|f| &f.name == field)
                        .map(|f| f.ty.clone());
                    self.cur_line = ctor.line;
                    let v = match &field_ty {
                        Some(ft) => self.gen_assigned_value(value, ft, &mut env, false, None),
                        None => self.gen_expr(value, &mut env, None),
                    };
                    if v == *field {
                        fields.push(field.clone());
                    } else {
                        fields.push(format!("{}: {}", field, v));
                    }
                }
                _ => {
                    self.cur_line = ctor.line;
                    self.err(
                        "构造器里只支持 `this.字段 = 值;` 形式的赋值",
                        "Rust 用结构体字面量一次完成构造：`Self { field: value }`；其它逻辑请放到普通方法里。",
                    );
                }
            }
        }
        if !phantom_params(class).is_empty() {
            fields.push("_marker: std::marker::PhantomData".to_string());
        }
        self.emit(&format!("Self {{ {} }}", fields.join(", ")));
        self.indent -= 1;
        self.emit("}");
    }

    fn gen_method(&mut self, m: &Method, in_trait_impl: bool, class_name: &str) {
        let vis = if in_trait_impl {
            ""
        } else if m.vis == Vis::Public {
            "pub "
        } else {
            ""
        };
        let throws = !m.throws.is_empty();
        if throws && in_trait_impl {
            self.cur_line = m.line;
            self.err_code(
                "JR3021",
                "接口实现的方法不能声明 throws",
                "接口签名里没有错误类型；把异常改成返回值，或在实现类里自己吞掉并处理。",
            );
        }
        let mut ps = Vec::new();
        if !m.is_static {
            ps.push("&self".to_string());
        }
        for p in &m.params {
            ps.push(format!("{}: {}", p.name, self.rust_type(&p.ty)));
        }
        let ret = if m.ret == Type::Void {
            if throws {
                format!(" -> Result<(), {}>", ERROR_ENUM)
            } else {
                String::new()
            }
        } else {
            let rt = self.rust_type(&m.ret);
            if throws {
                format!(" -> Result<{}, {}>", rt, ERROR_ENUM)
            } else {
                format!(" -> {}", rt)
            }
        };
        self.marker(m.line);
        self.emit(&format!(
            "{}fn {}{}({}){} {{",
            vis,
            m.name,
            impl_param_decl(&m.type_params),
            ps.join(", "),
            ret
        ));
        self.indent += 1;
        let prev_generic = self.generic_params.clone();
        self.generic_params.extend(m.type_params.iter().map(|p| p.name.clone()));
        let mut env = Env::new(m.ret.clone());
        for p in &m.params {
            env.vars.insert(p.name.clone(), p.ty.clone());
        }
        env.mutated = collect_mutated(&m.body);
        env.in_throws = throws;
        env.self_class = Some(class_name.to_string());
        let prev = self.in_method;
        self.in_method = !m.is_static;
        self.gen_stmts(&m.body, &mut env);
        self.in_method = prev;
        self.generic_params = prev_generic;
        if throws && m.ret == Type::Void {
            self.emit("Ok(())");
        }
        self.indent -= 1;
        self.emit("}");
    }

    // ---------- 语句 ----------

    fn gen_stmts(&mut self, stmts: &[Stmt], env: &mut Env) {
        for s in stmts {
            self.gen_stmt(s, env);
        }
    }

    fn gen_stmt(&mut self, stmt: &Stmt, env: &mut Env) {
        match stmt {
            Stmt::Let {
                ty,
                name,
                init,
                line,
            } => {
                self.cur_line = *line;
                self.marker(*line);
                if let Expr::Unary {
                    op: UnOp::Move,
                    expr,
                } = init
                {
                    if let Expr::Ident(src) = &**expr {
                        self.emit(&format!(
                            "// jrust 提示: 显式 `move` —— 所有权从 `{}` 搬到 `{}`，不复制。",
                            src, name
                        ));
                    }
                }
                let ty_str = self.rust_type(ty);
                let init_str = self.gen_assigned_value(init, ty, env, true, Some(name));
                let mut_kw = if env.mutated.contains(name) { "mut " } else { "" };
                self.emit(&format!(
                    "let {}{}: {} = {};",
                    mut_kw, name, ty_str, init_str
                ));
                env.vars.insert(name.clone(), ty.clone());
            }
            Stmt::Assign {
                target,
                value,
                line,
            } => {
                self.cur_line = *line;
                self.marker(*line);
                let target_ty = self.target_type(target, env);
                let t = self.gen_expr(target, env, None);
                let v = match &target_ty {
                    Some(tt) => self.gen_assigned_value(value, tt, env, true, Some(&t)),
                    None => self.gen_expr(value, env, None),
                };
                self.emit(&format!("{} = {};", t, v));
            }
            Stmt::Expr { expr, line } => {
                self.cur_line = *line;
                self.marker(*line);
                let e = self.gen_expr(expr, env, None);
                self.emit(&format!("{};", e));
            }
            Stmt::Return { value, line } => {
                self.cur_line = *line;
                self.marker(*line);
                match value {
                    Some(e) => {
                        let ret = env.ret.clone();
                        // 返回 `this` 的字段时从 `&self` 搬不走，必须复制。
                        let copy = self.is_self_field(e, env);
                        let v = self.gen_assigned_value(e, &ret, env, copy, None);
                        if env.in_throws {
                            self.emit(&format!("return Ok({});", v));
                        } else {
                            self.emit(&format!("return {};", v));
                        }
                    }
                    None => {
                        if env.in_throws {
                            self.emit("return Ok(());");
                        } else {
                            self.emit("return;");
                        }
                    }
                }
            }
            Stmt::If {
                cond,
                then,
                els,
                line,
            } => {
                self.cur_line = *line;
                self.marker(*line);
                let c = self.gen_expr(cond, env, None);
                self.emit(&format!("if {} {{", c));
                self.indent += 1;
                self.gen_stmts(then, env);
                self.indent -= 1;
                if let Some(else_body) = els {
                    self.emit("} else {");
                    self.indent += 1;
                    self.gen_stmts(else_body, env);
                    self.indent -= 1;
                    self.emit("}");
                } else {
                    self.emit("}");
                }
            }
            Stmt::While { cond, body, line } => {
                self.cur_line = *line;
                self.marker(*line);
                let c = self.gen_expr(cond, env, None);
                self.emit(&format!("while {} {{", c));
                self.indent += 1;
                self.gen_stmts(body, env);
                self.indent -= 1;
                self.emit("}");
            }
            Stmt::ForEach {
                ty,
                name,
                iter,
                body,
                line,
            } => {
                self.cur_line = *line;
                self.marker(*line);
                let it = self.gen_expr(iter, env, None);
                self.emit(&format!("for {} in &{} {{", name, it));
                self.indent += 1;
                env.vars.insert(name.clone(), ty.clone());
                self.gen_stmts(body, env);
                self.indent -= 1;
                self.emit("}");
            }
            Stmt::ForRange {
                ty,
                name,
                start,
                end,
                inclusive,
                descending,
                body,
                line,
            } => {
                self.cur_line = *line;
                self.marker(*line);
                let s = self.gen_expr(start, env, Some(ty));
                let e = self.gen_expr(end, env, Some(ty));
                // 升序：start..end / start..=end；降序要交换上下界再 rev()。
                let range = if *descending {
                    if *inclusive {
                        format!("({}..={}).rev()", e, s)
                    } else {
                        format!("((({}) + 1)..={}).rev()", e, s)
                    }
                } else if *inclusive {
                    format!("{}..={}", s, e)
                } else {
                    format!("{}..{}", s, e)
                };
                self.emit(&format!("for {} in {} {{", name, range));
                self.indent += 1;
                env.vars.insert(name.clone(), ty.clone());
                self.gen_stmts(body, env);
                self.indent -= 1;
                self.emit("}");
            }
            Stmt::Break { line } => {
                self.cur_line = *line;
                self.marker(*line);
                self.emit("break;");
            }
            Stmt::Continue { line } => {
                self.cur_line = *line;
                self.marker(*line);
                self.emit("continue;");
            }
            Stmt::Throw { expr, line } => {
                self.cur_line = *line;
                self.marker(*line);
                if !env.in_throws {
                    self.err_code(
                        "JR3021",
                        "`throw` 只能出现在声明了 `throws` 的方法里",
                        "在方法签名后加 `throws 错误名`，该方法就会返回 Result；或用 `return` 返回普通值。",
                    );
                    return;
                }
                let value = match expr {
                    Expr::New { class, args, .. } => {
                        let variant = simple_type_name(class);
                        if !self.error_types.iter().any(|n| n == &variant) {
                            self.err_code(
                                "JR3021",
                                format!("`throw {}` 的类型没有出现在任何 throws 列表里", class),
                                format!(
                                    "在方法签名里声明 `throws {}`，或改用已声明的错误类型（当前可用：{}）。",
                                    class,
                                    self.error_types.join(", ")
                                ),
                            );
                            return;
                        }
                        let msg = match args.len() {
                            0 => "String::new()".to_string(),
                            1 => {
                                let a = self.gen_expr(&args[0], env, None);
                                format!("({}).to_string()", a)
                            }
                            _ => {
                                self.err_code(
                                    "JR3021",
                                    "`throw new` 只支持 0 或 1 个参数",
                                    "错误枚举的每个分支只携带一条消息；需要更多信息时把它拼进字符串。",
                                );
                                "String::new()".to_string()
                            }
                        };
                        format!("{}::{}({})", ERROR_ENUM, variant, msg)
                    }
                    other => self.gen_expr(other, env, None),
                };
                self.emit(&format!("return Err({});", value));
            }
            Stmt::Switch {
                subject,
                cases,
                default,
                line,
            } => {
                self.cur_line = *line;
                self.marker(*line);
                self.gen_switch(subject, cases, default, env);
            }
        }
    }

    /// `switch` → Rust `match`，并做穷尽性 / fallthrough 诊断。
    fn gen_switch(
        &mut self,
        subject: &Expr,
        cases: &[SwitchCase],
        default: &Option<Vec<Stmt>>,
        env: &mut Env,
    ) {
        let sty = self.infer_type(subject, env);
        let enum_name = match &sty {
            Some(Type::Named(n)) | Some(Type::Generic(n, _)) if self.enums.contains_key(n) => {
                Some(n.clone())
            }
            _ => None,
        };
        let subject_is_bool = matches!(sty, Some(Type::Boolean));
        if enum_name.is_none() && !subject_is_bool {
            let msg = match &sty {
                Some(t) => format!(
                    "switch 只能用于 enum 或 boolean，这里是 `{}`",
                    self.rust_type(t)
                ),
                None => "无法推断 switch 条件表达式的类型（只支持 enum / boolean）".to_string(),
            };
            self.err_code(
                "JR3019",
                msg,
                "Rust 的 `match` 要求覆盖全部可能值。int / String 的 switch 请改成 if / else if 链。",
            );
            return;
        }

        let subj = self.gen_expr(subject, env, None);
        self.emit(&format!("match {} {{", subj));
        self.indent += 1;

        let mut covered: HashSet<String> = HashSet::new();
        let mut fallthrough_lines: Vec<usize> = Vec::new();
        for (idx, c) in cases.iter().enumerate() {
            self.cur_line = c.line;
            let pat = match &c.pattern {
                CasePattern::Bool(b) => {
                    covered.insert(b.to_string());
                    b.to_string()
                }
                CasePattern::Variant {
                    enum_name: qual,
                    name,
                    bindings,
                } => {
                    let Some(en) = &enum_name else {
                        self.err_code(
                            "JR3019",
                            format!("`case {}` 不是合法的枚举分支", name),
                            "枚举 switch 写 `case 变体(绑定):` 或 `case 枚举.变体(绑定):`。",
                        );
                        continue;
                    };
                    if let Some(q) = qual {
                        if q != en {
                            self.err_code(
                                "JR3019",
                                format!("`case {}.{}` 与 switch 条件类型 `{}` 不匹配", q, name, en),
                                format!("把限定名改成 `{}.{}`，或直接省略枚举名。", en, name),
                            );
                            continue;
                        }
                    }
                    let variants = &self.enums.get(en).unwrap().variants;
                    let Some(v) = variants.iter().find(|v| &v.name == name) else {
                        self.err_code(
                            "JR3019",
                            format!("枚举 `{}` 没有分支 `{}`", en, name),
                            format!(
                                "可选分支：{}。",
                                variants
                                    .iter()
                                    .map(|v| v.name.clone())
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            ),
                        );
                        continue;
                    };
                    if v.params.len() != bindings.len() {
                        self.err_code(
                            "JR3019",
                            format!(
                                "分支 `{}` 需要 {} 个绑定，实际写了 {} 个",
                                name,
                                v.params.len(),
                                bindings.len()
                            ),
                            "把 `case 变体(绑定名...)` 的绑定数量与 enum 声明对齐。",
                        );
                        continue;
                    }
                    covered.insert(name.clone());
                    if bindings.is_empty() {
                        format!("{}::{}", en, name)
                    } else {
                        format!("{}::{}({})", en, name, bindings.join(", "))
                    }
                }
                CasePattern::Other(e) => {
                    let lit = self.gen_expr(e, env, None);
                    self.err_code(
                        "JR3019",
                        format!("枚举 / boolean 的 switch 里出现了字面量 case `{}`", lit),
                        "枚举分支写 `case 变体名(...)`；boolean 写 `case true:` / `case false:`。",
                    );
                    continue;
                }
            };

            let guard = match &c.guard {
                Some(g) => format!(" if {}", self.gen_expr(g, env, None)),
                None => String::new(),
            };
            self.marker(c.line);
            self.emit(&format!("{}{} => {{", pat, guard));
            self.indent += 1;

            let is_last = idx + 1 == cases.len() && default.is_none();
            let body = strip_trailing_break(&c.body);
            if !is_last && !matches!(c.body.last(), Some(Stmt::Break { .. })) {
                fallthrough_lines.push(c.line);
            }
            self.gen_stmts(body, env);
            self.indent -= 1;
            self.emit("}");
        }

        let mut missing: Vec<String> = Vec::new();
        match &enum_name {
            Some(en) => {
                for v in &self.enums.get(en).unwrap().variants {
                    if !covered.contains(&v.name) {
                        missing.push(v.name.clone());
                    }
                }
            }
            None => {
                for b in ["true", "false"] {
                    if !covered.contains(b) {
                        missing.push(b.to_string());
                    }
                }
            }
        }
        if default.is_some() {
            let db = default.as_ref().unwrap();
            self.emit("_ => {");
            self.indent += 1;
            self.gen_stmts(strip_trailing_break(db), env);
            self.indent -= 1;
            self.emit("}");
            missing.clear();
        }
        if !missing.is_empty() {
            self.err_code(
                "JR3019",
                format!("switch 没有覆盖全部可能值，缺少：{}", missing.join(", ")),
                "补上缺少的 case，或加 `default:` 分支（产物里生成 `_ =>`）。",
            );
        }
        for line in fallthrough_lines {
            self.cur_line = line;
            self.err_code(
                "JR3020",
                "case 块没有 break，会产生 fallthrough",
                "Rust 的 match 分支不会穿透：给该分支补 `break;`，或把相邻分支合并成一个分支。",
            );
        }

        self.indent -= 1;
        self.emit("}");
    }

    // ---------- 表达式 ----------

    fn gen_expr(&mut self, e: &Expr, env: &mut Env, expect: Option<&Type>) -> String {
        match e {
            Expr::Int(n) => n.to_string(),
            Expr::Float(f) => fmt_float(*f),
            Expr::Str(s) => {
                let lit = format!("{:?}", s);
                if let Some(t) = expect {
                    if *t == Type::String {
                        return format!("{}.to_string()", lit);
                    }
                }
                lit
            }
            Expr::Char(c) => format!("{:?}", c),
            Expr::Bool(b) => b.to_string(),
            Expr::Null => match expect {
                Some(Type::Nullable(_)) => "None".to_string(),
                Some(t) => {
                    let tn = self.rust_type(t);
                    self.err_code(
                        "JR3005",
                        format!("`null` 只能赋给可空类型，这里的目标类型是 `{}`", tn),
                        format!(
                            "把声明 / 参数 / 返回类型写成 `{}?`（→ Option<{}>），或改用一个非空值。",
                            tn, tn
                        ),
                    );
                    "None".to_string()
                }
                None => {
                    self.err_code(
                        "JR3005",
                        "`null` 只能出现在可空类型上下文",
                        "声明写 `T? x = null;`，比较写 `x == null` / `x != null`；其余位置请给出非空值。",
                    );
                    "None".to_string()
                }
            },
            Expr::Ident(n) => n.clone(),
            Expr::This => "self".to_string(),
            Expr::Field(recv, name) => {
                if let Expr::Ident(en) = &**recv {
                    if let Some(cd) = self.classes.get(en) {
                        if cd.fields.iter().any(|f| &f.name == name && is_const_field(f)) {
                            // 静态常量在产物里是模块级 `const`：`Cls.N` → `N`。
                            return name.clone();
                        }
                    }
                    if let Some(ed) = self.enums.get(en) {
                        if ed
                            .variants
                            .iter()
                            .any(|v| &v.name == name && v.params.is_empty())
                        {
                            return format!("{}::{}", en, name);
                        }
                    }
                }
                let r = self.gen_expr(recv, env, None);
                format!("{}.{}", r, name)
            }
            Expr::Call { recv, name, args } => self.gen_call(recv, name, args, env, expect),
            Expr::New {
                class,
                type_args,
                args,
            } => {
                let inner = self.gen_new(class, type_args, args, env, expect);
                match expect {
                    Some(Type::Named(t)) if self.ifaces.contains(t) => {
                        return format!("Box::new({})", inner);
                    }
                    Some(Type::Generic(t, _)) if self.ifaces.contains(t) => {
                        return format!("Box::new({})", inner);
                    }
                    _ => {}
                }
                inner
            }
            Expr::NewArray { elem, len } => {
                let default = self.array_default(elem);
                // 字面量长度直接写数字，避免产物里出现多余的 `as usize`。
                let n = match &**len {
                    Expr::Int(k) if *k >= 0 => k.to_string(),
                    _ => format!("({}) as usize", self.gen_expr(len, env, Some(&Type::Int))),
                };
                format!("vec![{}; {}]", default, n)
            }
            Expr::Binary { op, lhs, rhs } => {
                if *op == BinOp::Add
                    && (self.expr_is_stringy(lhs, env) || self.expr_is_stringy(rhs, env))
                {
                    return self.gen_concat(&[lhs, rhs], env);
                }
                if matches!(op, BinOp::Eq | BinOp::Ne) {
                    let l_null = matches!(strip_parens_expr(lhs), Expr::Null);
                    let r_null = matches!(strip_parens_expr(rhs), Expr::Null);
                    if l_null && r_null {
                        return (*op == BinOp::Eq).to_string();
                    }
                    if l_null || r_null {
                        let other = if l_null { rhs } else { lhs };
                        if let Some(t) = self.infer_type(other, env) {
                            if !matches!(t, Type::Nullable(_)) {
                                let tn = self.rust_type(&t);
                                self.err_code(
                                    "JR3005",
                                    format!("`{}` 不是可空类型，不能与 `null` 比较", tn),
                                    "只有 `T?`（→ Option<T>）才能判空；普通值请直接使用或比较其内容。",
                                );
                            }
                        }
                        let x = self.gen_expr(other, env, None);
                        let m = if *op == BinOp::Eq { "is_none" } else { "is_some" };
                        return format!("{}.{}()", x, m);
                    }
                }
                let l = self.gen_expr(lhs, env, None);
                let r = self.gen_expr(rhs, env, None);
                format!("{} {} {}", l, binop_str(*op), r)
            }
            Expr::Unary { op, expr } => match op {
                UnOp::Not => format!("!{}", self.gen_expr(expr, env, None)),
                UnOp::Neg => format!("-{}", self.gen_expr(expr, env, None)),
                UnOp::Move => self.gen_move(expr, env),
            },
            Expr::Index(a, b) => {
                let x = self.gen_expr(a, env, None);
                let y = self.gen_expr(b, env, None);
                // Rust 的索引必须是 usize；字面量交给编译器推断，变量统一转。
                let idx = match &**b {
                    Expr::Int(k) if *k >= 0 => y,
                    _ => format!("({}) as usize", y),
                };
                format!("{}[{}]", x, idx)
            }
            Expr::Paren(inner) => {
                let x = self.gen_expr(inner, env, expect);
                format!("({})", x)
            }
            Expr::Closure { params, body } => {
                let mut ps = Vec::new();
                for p in params {
                    match &p.ty {
                        Some(t) => ps.push(format!("{}: {}", p.name, self.rust_type(t))),
                        None => ps.push(p.name.clone()),
                    }
                }
                let b = self.gen_expr(body, env, None);
                format!("|{}| {}", ps.join(", "), b)
            }
            Expr::Try(inner) => {
                if !env.in_throws {
                    self.err_code(
                        "JR3018",
                        "`?` 只能用在声明了 throws 的方法里",
                        "给当前方法签名加 `throws 错误名`（返回 Result 后 `?` 才合法），或在调用处显式处理返回值。",
                    );
                }
                let x = self.gen_expr(inner, env, None);
                format!("{}?", x)
            }
        }
    }

    /// `move x`：产物就是 `x`（Rust 的赋值/传参本身就是 move），
    /// 但会校验目标是否合法，并对 Copy 类型给出教学性报错。
    fn gen_move(&mut self, inner: &Expr, env: &mut Env) -> String {
        match inner {
            Expr::Ident(n) => {
                let info = env
                    .vars
                    .get(n)
                    .map(|t| (t.is_copy_scalar(), self.rust_type(t)));
                match info {
                    Some((true, tn)) => {
                        self.err(
                            format!("`move {}`：`{}` 的类型是 {}，属于 Copy 标量", n, n, tn),
                            "i32 / f64 / bool / char 赋值时自动复制，没有所有权可搬；`move` 只用于 String、Vec、struct 等拥有堆数据的类型。",
                        );
                    }
                    None => {
                        self.err(
                            format!("`move {}`：变量 `{}` 未声明", n, n),
                            "先声明变量，再搬走它的所有权。",
                        );
                    }
                    _ => {}
                }
                self.gen_expr(inner, env, None)
            }
            Expr::Field(recv, f) => {
                if matches!(**recv, Expr::This) {
                    self.err(
                        format!("`move this.{}`：方法的 `self` 是借用（`&self`），字段搬不出去", f),
                        format!(
                            "要整个值就 move 拥有者本身；只是要字段的值就写 `this.{}.clone()`。",
                            f
                        ),
                    );
                }
                self.gen_expr(inner, env, None)
            }
            Expr::Index(..) => {
                self.err(
                    "`move` 不能作用于下标：Rust 不允许从 `Vec` 下标搬走所有权",
                    "元素想拿走用 `v.remove(i)`；只是要值就写 `v[i].clone()`。",
                );
                self.gen_expr(inner, env, None)
            }
            _ => {
                self.err(
                    "`move` 只能作用于变量、字段或下标",
                    "字面量和临时表达式没有可搬走的所有权；`move` 的目标必须是已经存在的值。",
                );
                self.gen_expr(inner, env, None)
            }
        }
    }

    /// Map 的键参数：String 键生成 `.as_str()`（`HashMap<String, V>::get` 接受 `&str`，
    /// 也兼容字符串字面量和 `&String`），其它键生成 `&expr`。
    fn gen_map_key(&mut self, e: Option<&Expr>, k_ty: &Type, env: &mut Env) -> String {
        match e {
            // 字符串字面量本身就是 &str，直接传；`"x".as_str()` 在新版标准库会撞到不稳定的 str_as_str。
            Some(Expr::Str(_)) => self.gen_expr(e.unwrap(), env, None),
            Some(e) if *k_ty == Type::String => {
                format!("{}.as_str()", self.gen_expr(e, env, None))
            }
            Some(e) => format!("&{}", self.gen_expr(e, env, Some(k_ty))),
            None => String::new(),
        }
    }

    fn gen_new(
        &mut self,
        class: &str,
        type_args: &[Type],
        args: &[Expr],
        env: &mut Env,
        expect: Option<&Type>,
    ) -> String {
        if matches!(class, "ArrayList" | "LinkedList" | "Vector") {
            if !args.is_empty() {
                self.err(
                    "jrust MVP 只支持无参的 new ArrayList<>()",
                    "Rust 用 `Vec::new()` 再逐个 `push`。",
                );
            }
            return "Vec::new()".to_string();
        }
        // 钻石 `new Box<>(..)` 的类型实参来自左侧声明类型（`Box<String> b`）。
        let mut targs: Vec<Type> = type_args.to_vec();
        if targs.is_empty() {
            if let Some(Type::Generic(c, inner)) = expect {
                if c == class {
                    targs = inner.clone();
                }
            }
        }
        let info: Option<(Vec<String>, Option<Vec<Type>>, bool, bool)> =
            self.classes.get(class).map(|c| {
                (
                    c.type_params.iter().map(|p| p.name.clone()).collect(),
                    c.ctor
                        .as_ref()
                        .map(|ct| ct.params.iter().map(|p| p.ty.clone()).collect()),
                    c.ctor.is_some(),
                    !phantom_params(c).is_empty(),
                )
            });
        let map: HashMap<String, Type> = info
            .as_ref()
            .map(|(params, _, _, _)| params.iter().cloned().zip(targs.iter().cloned()).collect())
            .unwrap_or_default();
        let ctor_params: Option<Vec<Type>> = info.as_ref().and_then(|(_, ps, _, _)| {
            ps.as_ref()
                .map(|ps| ps.iter().map(|p| subst_type(p, &map)).collect())
        });
        let mut arg_strs = Vec::new();
        for (i, a) in args.iter().enumerate() {
            match ctor_params.as_ref().and_then(|p| p.get(i)) {
                Some(pt) => arg_strs.push(self.gen_arg(a, pt, env)),
                None => arg_strs.push(self.gen_expr(a, env, None)),
            }
        }
        // 显式类型实参才写 turbofish；钻石交给类型推断。
        let turbofish = if type_args.is_empty() {
            String::new()
        } else {
            let ts: Vec<String> = type_args.iter().map(|t| self.rust_type(t)).collect();
            format!("::<{}>", ts.join(", "))
        };
        let Some((_, _, has_ctor, needs_phantom)) = info else {
            return format!("{}{}::new({})", class, turbofish, arg_strs.join(", "));
        };
        if has_ctor {
            format!("{}{}::new({})", class, turbofish, arg_strs.join(", "))
        } else if args.is_empty() {
            if needs_phantom {
                format!("{} {{ _marker: std::marker::PhantomData }}", class)
            } else {
                class.to_string()
            }
        } else {
            self.err(
                format!("类 {} 没有构造器却传入了参数", class),
                "为它写一个构造器（方法名与类名相同）；Rust 里对应 `impl X { pub fn new(...) -> Self }`。",
            );
            class.to_string()
        }
    }

    fn gen_call(
        &mut self,
        recv: &Option<Box<Expr>>,
        name: &str,
        args: &[Expr],
        env: &mut Env,
        expect: Option<&Type>,
    ) -> String {
        if (name == "println" || name == "print") && recv.as_ref().map_or(true, |r| is_system_out(r)) {
            let mac = if name == "println" { "println" } else { "print" };
            return self.gen_print_macro(args, env, mac);
        }

        // `Shape.Circle(2.0)` → `Shape::Circle(2.0)`：枚举分支构造。
        if let Some(Expr::Ident(en)) = recv.as_deref() {
            let vparams: Option<Vec<Type>> = self.enums.get(en).and_then(|ed| {
                ed.variants
                    .iter()
                    .find(|v| &v.name == name)
                    .map(|v| v.params.iter().map(|p| p.ty.clone()).collect())
            });
            if let Some(vparams) = vparams {
                // 泛型枚举：`Maybe<String> m = Maybe.Some("x")` 时按声明类型替换分支形参。
                let vmap: HashMap<String, Type> = self
                    .enums
                    .get(en)
                    .map(|ed| {
                        let args = match expect {
                            Some(Type::Generic(c, a)) if c == en => a.clone(),
                            _ => Vec::new(),
                        };
                        ed.type_params.iter().map(|p| p.name.clone()).zip(args).collect()
                    })
                    .unwrap_or_default();
                let vparams: Vec<Type> = vparams.iter().map(|t| subst_type(t, &vmap)).collect();
                let nparams = vparams.len();
                if nparams != args.len() {
                    self.err_code(
                        "JR3019",
                        format!(
                            "枚举分支 `{}::{}` 需要 {} 个参数，实际给了 {} 个",
                            en,
                            name,
                            nparams,
                            args.len()
                        ),
                        "构造带数据枚举分支时，参数数量必须与 enum 声明一致。",
                    );
                }
                let mut vals = Vec::new();
                for (i, a) in args.iter().enumerate() {
                    match vparams.get(i) {
                        Some(pt) => vals.push(self.gen_arg(a, pt, env)),
                        None => vals.push(self.gen_expr(a, env, None)),
                    }
                }
                return if vals.is_empty() {
                    format!("{}::{}", en, name)
                } else {
                    format!("{}::{}({})", en, name, vals.join(", "))
                };
            }
        }

        let recv_ty = recv.as_ref().and_then(|r| self.infer_type(r, env));
        if let Some(Type::Nullable(inner)) = &recv_ty {
            let tn = self.rust_type(inner);
            self.err_code(
                "JR3006",
                format!("`{}?` 是可能为空的值，不能直接调用 `{}`", tn, name),
                format!(
                    "先判空再取出来：`if (x != null) {{ {} v = x; v.{}(...) }}`（→ `if let Some(v) = x`）。",
                    tn, name
                ),
            );
        }
        if let Some(Type::Map(k_ty, v_ty)) = &recv_ty {
            let r = self.gen_expr(recv.as_ref().unwrap(), env, None);
            match name {
                "size" => return format!("{}.len()", r),
                "isEmpty" => return format!("{}.is_empty()", r),
                "put" => {
                    if args.len() != 2 {
                        self.err(
                            "put 需要键和值两个参数",
                            "例如 `m.put(\"a\", 1)`；Rust 对应 `insert`。",
                        );
                    }
                    let k = args
                        .first()
                        .map(|a| self.gen_arg(a, k_ty, env))
                        .unwrap_or_default();
                    let v = args
                        .get(1)
                        .map(|a| self.gen_arg(a, v_ty, env))
                        .unwrap_or_default();
                    return format!("{}.insert({}, {})", r, k, v);
                }
                "getOrNull" => {
                    if args.len() != 1 {
                        self.err("getOrNull 需要一个键", "例如 `m.getOrNull(\"a\")`。");
                    }
                    let k = self.gen_map_key(args.first(), k_ty, env);
                    return format!("{}.get({}).cloned()", r, k);
                }
                "get" => {
                    if args.len() != 1 {
                        self.err("get 需要一个键", "例如 `m.get(\"a\")`。");
                    }
                    let k = self.gen_map_key(args.first(), k_ty, env);
                    // Rust 的 get 返回 Option<&V>；键缺失时 Java 得到 null，jrust 没有 null，
                    // 这里取拥有所有权的值，缺失即 panic（可用 containsKey 先判断）。
                    return format!("{}.get({}).cloned().unwrap()", r, k);
                }
                "containsKey" => {
                    if args.len() != 1 {
                        self.err("containsKey 需要一个键", "例如 `m.containsKey(\"a\")`。");
                    }
                    let k = self.gen_map_key(args.first(), k_ty, env);
                    return format!("{}.contains_key({})", r, k);
                }
                other => {
                    self.err(
                        format!("Map 没有方法 `{}`", other),
                        "MVP 支持 put / get / getOrNull / size / containsKey / isEmpty。",
                    );
                    return format!("{}.{}()", r, other);
                }
            }
        }
        // `T[]`（`new T[n]`）与 `List<T>` 在产物里都是 `Vec`，方法映射共享。
        let is_list = matches!(recv_ty, Some(Type::List(_)) | Some(Type::Array(_)));
        let elem_is_trait = match &recv_ty {
            Some(Type::List(inner) | Type::Array(inner)) => match &**inner {
                Type::Named(n) | Type::Generic(n, _) => self.ifaces.contains(n),
                _ => false,
            },
            _ => false,
        };
        let elem_is_string = match &recv_ty {
            Some(Type::List(inner) | Type::Array(inner)) => matches!(&**inner, Type::String),
            _ => false,
        };
        let elem_ty = match &recv_ty {
            Some(Type::List(inner) | Type::Array(inner)) => Some((**inner).clone()),
            _ => None,
        };
        let param_types = self.call_param_types(recv, name, env);

        let mut arg_strs = Vec::new();
        for (i, a) in args.iter().enumerate() {
            if is_list && name == "add" {
                if let Some(et) = &elem_ty {
                    if matches!(et, Type::Nullable(_)) {
                        arg_strs.push(self.gen_arg(a, et, env));
                        continue;
                    }
                }
                let inner = self.gen_expr(a, env, None);
                if elem_is_trait {
                    arg_strs.push(format!("Box::new({})", inner));
                } else if elem_is_string {
                    // Vec<String> 的 push 需要 String，字面量要补 to_string()
                    arg_strs.push(format!("({}).to_string()", inner));
                } else {
                    arg_strs.push(inner);
                }
            } else {
                match param_types.as_ref().and_then(|p| p.get(i)) {
                    Some(pt) => arg_strs.push(self.gen_arg(a, pt, env)),
                    None => arg_strs.push(self.gen_expr(a, env, None)),
                }
            }
        }

        if is_list && name == "get" && !arg_strs.is_empty() {
            let r = self.gen_expr(recv.as_ref().unwrap(), env, None);
            return format!("{}[{}]", r, arg_strs[0]);
        }

        let mapped = match (is_list, name) {
            (true, "add") => "push",
            (true, "size") => "len",
            (_, "length") => "len",
            _ => name,
        };

        match recv {
            Some(r) => {
                let rs = self.gen_expr(r, env, None);
                format!("{}.{}({})", rs, mapped, arg_strs.join(", "))
            }
            None => {
                if self.in_method {
                    format!("self.{}({})", mapped, arg_strs.join(", "))
                } else {
                    format!("{}({})", mapped, arg_strs.join(", "))
                }
            }
        }
    }

    /// 把 `System.out.println(...)` 转成 `println!`。
    /// 参数是 `+` 串联的字符串时，自动展开成格式串：`"a" + x` → `"a{}"` + `x`。
    fn gen_print_macro(&mut self, args: &[Expr], env: &mut Env, mac: &str) -> String {
        if args.is_empty() {
            return format!("{}!()", mac);
        }
        if args.len() > 1 {
            let fmt = match &args[0] {
                Expr::Str(s) => s,
                _ => {
                    self.err(
                        format!("{} 多参数时第一个参数必须是格式串字面量", mac),
                        format!(
                            "例如 {}(\"{{}} area={{}}\", s.name(), s.area())；单参数仍可写 \"a\" + b 拼接。",
                            mac
                        ),
                    );
                    let parts: Vec<String> =
                        args.iter().map(|a| self.gen_expr(a, env, None)).collect();
                    return format!("{}!(\"{{:?}}\", {})", mac, parts.join(", "));
                }
            };
            let holes = match count_placeholders(fmt) {
                Some(n) => n,
                None => {
                    self.err(
                        "格式串里的 `{` / `}` 要成对写",
                        "字面量花括号写成 `{{` / `}}`，例如 println!(\"{{}}\")。",
                    );
                    return format!("{}!(\"{}\")", mac, escape_str_literal(fmt));
                }
            };
            if holes != args.len() - 1 {
                self.err(
                    format!(
                        "格式串有 {} 个 `{{}}`，但给了 {} 个参数",
                        holes,
                        args.len() - 1
                    ),
                    "占位符与参数按顺序一一对应。",
                );
            }
            for a in &args[1..] {
                if self.expr_is_nullable(a, env) {
                    self.err(
                        "不能直接打印可空类型 `T?`",
                        "先判空并取出非空值：`if (x != null) { T v = x; println(v); }`。",
                    );
                }
            }
            let rest: Vec<String> = args[1..]
                .iter()
                .map(|a| self.gen_expr(a, env, None))
                .collect();
            return format!("{}!(\"{}\", {})", mac, escape_str_literal(fmt), rest.join(", "));
        }
        if args.len() != 1 {
            let parts: Vec<String> = args.iter().map(|a| self.gen_expr(a, env, None)).collect();
            self.err(
                format!("{} 只接受一个参数（jrust MVP）", mac),
                "先把多个值用 + 拼成一个字符串，再打印。",
            );
            return format!("{}!(\"{{:?}}\", {})", mac, parts.join(", "));
        }
        let arg = &args[0];
        if self.expr_is_nullable(arg, env) {
            self.err(
                "不能直接打印可空类型 `T?`",
                "先判空并取出非空值：`if (x != null) { T v = x; println(v); }`。",
            );
        }
        let mut parts: Vec<&Expr> = Vec::new();
        flatten_add_chain(arg, &mut parts);
        let has_str = parts.iter().any(|p| matches!(p, Expr::Str(_)));

        if has_str {
            let mut tmpl = String::new();
            let mut params: Vec<String> = Vec::new();
            for p in &parts {
                match p {
                    Expr::Str(s) => tmpl.push_str(&escape_in_fmt(s)),
                    other => {
                        tmpl.push_str("{}");
                        params.push(self.gen_expr(other, env, None));
                    }
                }
            }
            if params.is_empty() {
                format!("{}!(\"{}\")", mac, tmpl)
            } else {
                format!("{}!(\"{}\", {})", mac, tmpl, params.join(", "))
            }
        } else if let Expr::Str(s) = arg {
            format!("{}!(\"{}\")", mac, escape_in_fmt(s))
        } else {
            let v = self.gen_expr(arg, env, None);
            format!("{}!(\"{{}}\", {})", mac, v)
        }
    }

    /// 把 `Named(C)` / `Generic(C, args)` 统一成「类型 C + 类型参数绑定」。
    /// C 可以是 class，也可以是 interface（泛型 trait）。
    fn type_binding(&self, ty: &Type) -> Option<(String, HashMap<String, Type>)> {
        match ty {
            Type::Named(c) => Some((c.clone(), HashMap::new())),
            Type::Generic(c, args) => {
                let names: Vec<String> = if let Some(cd) = self.classes.get(c) {
                    cd.type_params.iter().map(|p| p.name.clone()).collect()
                } else {
                    self.prog
                        .interfaces
                        .iter()
                        .find(|i| &i.name == c)?
                        .type_params
                        .iter()
                        .map(|p| p.name.clone())
                        .collect()
                };
                let map = names.into_iter().zip(args.iter().cloned()).collect();
                Some((c.clone(), map))
            }
            _ => None,
        }
    }

    fn infer_type(&self, e: &Expr, env: &Env) -> Option<Type> {
        match e {
            Expr::Ident(n) => env.vars.get(n).cloned(),
            Expr::This => env.self_class.clone().map(Type::Named),
            Expr::New {
                class, type_args, ..
            } => {
                if type_args.is_empty() {
                    Some(Type::Named(class.clone()))
                } else {
                    Some(Type::Generic(class.clone(), type_args.clone()))
                }
            }
            Expr::NewArray { elem, .. } => Some(Type::Array(Box::new(elem.clone()))),
            Expr::Field(recv, f) => {
                if let Expr::Ident(en) = &**recv {
                    if let Some(cd) = self.classes.get(en) {
                        if let Some(x) = cd.fields.iter().find(|x| &x.name == f && is_const_field(x)) {
                            return Some(x.ty.clone());
                        }
                    }
                    if self
                        .enums
                        .get(en)
                        .is_some_and(|ed| ed.variants.iter().any(|v| &v.name == f))
                    {
                        return Some(Type::Named(en.clone()));
                    }
                }
                if let Some((cname, map)) = self.infer_type(recv, env).and_then(|t| self.type_binding(&t)) {
                    if let Some(cd) = self.classes.get(&cname) {
                        return instance_fields(cd)
                            .find(|x| &x.name == f)
                            .map(|x| subst_type(&x.ty, &map));
                    }
                }
                None
            }
            Expr::Call { recv, name, .. } => {
                // 枚举分支构造：`Shape.Circle(..)` → Shape
                if let Some(Expr::Ident(en)) = recv.as_deref() {
                    if let Some(ed) = self.enums.get(en) {
                        if ed.variants.iter().any(|v| &v.name == name) {
                            return Some(Type::Named(en.clone()));
                        }
                    }
                    // 静态调用 `Cls.method(..)`
                    if let Some(cd) = self.classes.get(en) {
                        if let Some(m) = cd.methods.iter().find(|m| &m.name == name && m.is_static)
                        {
                            return Some(m.ret.clone());
                        }
                    }
                }
                let recv_ty = recv.as_deref().and_then(|r| self.infer_type(r, env));
                if let Some(Type::Map(_, v)) = &recv_ty {
                    if name.as_str() == "getOrNull" {
                        return Some(Type::Nullable(v.clone()));
                    }
                }
                if let Some(rt) = &recv_ty {
                    if let Some((c, map)) = self.type_binding(rt) {
                        if let Some(cd) = self.classes.get(&c) {
                            if let Some(m) =
                                cd.methods.iter().find(|m| m.name == *name && !m.is_static)
                            {
                                return Some(subst_type(&m.ret, &map));
                            }
                        }
                        if let Some(iface) = self.prog.interfaces.iter().find(|i| i.name == c) {
                            if let Some(m) = iface.methods.iter().find(|m| m.name == *name) {
                                return Some(subst_type(&m.ret, &map));
                            }
                        }
                    }
                }
                if recv.is_none() {
                    if let Some(c) = &env.self_class {
                        if let Some(m) = self.classes.get(c).and_then(|cd| {
                            cd.methods.iter().find(|m| m.name == *name && m.is_static)
                        }) {
                            return Some(m.ret.clone());
                        }
                    }
                }
                None
            }
            Expr::Paren(inner) | Expr::Try(inner) => self.infer_type(inner, env),
            Expr::Unary {
                op: UnOp::Move,
                expr,
            } => self.infer_type(expr, env),
            _ => None,
        }
    }
}

// ---------- 自由函数 ----------

/// `Named` / `Generic` 的类型名（不含实参）。
fn type_name(ty: &Type) -> &str {
    match ty {
        Type::Named(n) | Type::Generic(n, _) => n,
        _ => "",
    }
}

/// 把类型里的类型参数（`T`）替换成实参类型（`Integer` → `Int`）。
fn subst_type(ty: &Type, map: &HashMap<String, Type>) -> Type {
    match ty {
        Type::Named(n) => map.get(n).cloned().unwrap_or_else(|| ty.clone()),
        Type::Generic(n, args) => {
            Type::Generic(n.clone(), args.iter().map(|a| subst_type(a, map)).collect())
        }
        Type::List(inner) => Type::List(Box::new(subst_type(inner, map))),
        Type::Map(k, v) => Type::Map(
            Box::new(subst_type(k, map)),
            Box::new(subst_type(v, map)),
        ),
        Type::Array(inner) => Type::Array(Box::new(subst_type(inner, map))),
        Type::Nullable(inner) => Type::Nullable(Box::new(subst_type(inner, map))),
        other => other.clone(),
    }
}

/// `<T, U>`；没有类型参数时返回空串。上界只出现在 impl / 方法位置，声明位置省略。
fn type_param_decl(params: &[TypeParam]) -> String {
    if params.is_empty() {
        String::new()
    } else {
        let ps: Vec<&str> = params.iter().map(|p| p.name.as_str()).collect();
        format!("<{}>", ps.join(", "))
    }
}

/// `<T: A + B + Clone>`：泛型类 / 方法的实现需要能按值复制字段和参数，
/// 因此用户上界之后统一追加 `Clone`（已写则不重复）。
fn impl_param_decl(params: &[TypeParam]) -> String {
    if params.is_empty() {
        String::new()
    } else {
        let ps: Vec<String> = params
            .iter()
            .map(|p| {
                let mut bounds: Vec<String> = p.bounds.iter().map(bound_text).collect();
                if !bounds.iter().any(|b| b == "Clone") {
                    bounds.push("Clone".to_string());
                }
                format!("{}: {}", p.name, bounds.join(" + "))
            })
            .collect();
        format!("<{}>", ps.join(", "))
    }
}

/// 泛型上界的 Rust 写法：`A` / `Comparable<T>`。
fn bound_text(ty: &Type) -> String {
    match ty {
        Type::Named(n) => n.clone(),
        Type::Generic(n, args) => {
            let inner: Vec<String> = args.iter().map(bound_text).collect();
            format!("{}<{}>", n, inner.join(", "))
        }
        Type::String => "String".to_string(),
        other => format!("{:?}", other),
    }
}

/// `static final` 字段：产物是模块级 `const`，不属于结构体（JR-D8）。
fn is_const_field(f: &Field) -> bool {
    f.is_static && f.is_final
}

fn instance_fields(class: &ClassDecl) -> impl Iterator<Item = &Field> {
    class.fields.iter().filter(|f| !is_const_field(f))
}

/// Rust 要求每个类型参数都被字段用到；没用到的用 PhantomData 占位（`class Box<T> {}`）。
fn phantom_params(class: &ClassDecl) -> Vec<String> {
    let mut used = HashSet::new();
    for f in instance_fields(class) {
        collect_type_params(&f.ty, &mut used);
    }
    class
        .type_params
        .iter()
        .filter(|p| !used.contains(&p.name))
        .map(|p| p.name.clone())
        .collect()
}

fn collect_type_params(ty: &Type, out: &mut HashSet<String>) {
    match ty {
        Type::Named(n) => {
            out.insert(n.clone());
        }
        Type::Generic(n, args) => {
            out.insert(n.clone());
            for a in args {
                collect_type_params(a, out);
            }
        }
        Type::List(inner) | Type::Array(inner) | Type::Nullable(inner) => {
            collect_type_params(inner, out)
        }
        Type::Map(k, v) => {
            collect_type_params(k, out);
            collect_type_params(v, out);
        }
        _ => {}
    }
}

/// 类型能否按值复制（用于 `#[derive(Clone)]` 与赋值处补 `.clone()`）。
/// `params` 是当前作用域的类型参数名（`T`），泛型上下文里它们按 `T: Clone` 处理。
fn type_cloneable(ty: &Type, named: &HashSet<String>, params: &HashSet<String>) -> bool {
    match ty {
        t if t.is_copy_scalar() => true,
        Type::Void => true,
        Type::String => true,
        Type::List(inner) | Type::Array(inner) => type_cloneable(inner, named, params),
        Type::Map(k, v) => type_cloneable(k, named, params) && type_cloneable(v, named, params),
        Type::Named(n) => named.contains(n) || params.contains(n),
        Type::Generic(n, args) => {
            named.contains(n) && args.iter().all(|a| type_cloneable(a, named, params))
        }
        Type::Nullable(inner) => type_cloneable(inner, named, params),
        _ => false,
    }
}

/// `java.io.IOException` → `IOException`
fn simple_type_name(name: &str) -> String {
    name.rsplit('.').next().unwrap_or(name).to_string()
}

/// `throws` 里的常见异常名映射到标准库错误类型，用于生成 `impl From<...>`。
fn std_error_source(name: &str) -> Option<&'static str> {
    match name {
        "IOException" | "IoError" | "UncheckedIOException" => Some("std::io::Error"),
        "ParseIntError" | "NumberFormatException" => Some("std::num::ParseIntError"),
        "ParseFloatError" => Some("std::num::ParseFloatError"),
        _ => None,
    }
}

/// 去掉 case 块结尾的 `break;`（它只表示「不穿透」，产物 match 分支天然不穿透）。
fn strip_trailing_break(body: &[Stmt]) -> &[Stmt] {
    match body.last() {
        Some(Stmt::Break { .. }) => &body[..body.len() - 1],
        _ => body,
    }
}

fn binop_str(op: BinOp) -> &'static str {
    match op {
        BinOp::Add => "+",
        BinOp::Sub => "-",
        BinOp::Mul => "*",
        BinOp::Div => "/",
        BinOp::Rem => "%",
        BinOp::Lt => "<",
        BinOp::Le => "<=",
        BinOp::Gt => ">",
        BinOp::Ge => ">=",
        BinOp::Eq => "==",
        BinOp::Ne => "!=",
        BinOp::And => "&&",
        BinOp::Or => "||",
    }
}

fn fmt_float(f: f64) -> String {
    if f.fract() == 0.0 && f.abs() < 1e16 {
        format!("{:.1}", f)
    } else {
        format!("{}", f)
    }
}

fn is_system_out(e: &Expr) -> bool {
    match e {
        Expr::Field(inner, f) if f == "out" => {
            matches!(&**inner, Expr::Ident(n) if n == "System")
        }
        _ => false,
    }
}

/// 去掉多余的括号（`(x)` 与 `x` 等价）。
fn strip_parens_expr(e: &Expr) -> &Expr {
    match e {
        Expr::Paren(inner) => strip_parens_expr(inner),
        _ => e,
    }
}

/// 变量 / 字段是值的「持有者」，按值语义需要复制。
fn is_holder_expr(e: &Expr) -> bool {
    matches!(strip_parens_expr(e), Expr::Ident(_) | Expr::Field(_, _))
}

fn flatten_add_chain<'e>(e: &'e Expr, out: &mut Vec<&'e Expr>) {
    match e {
        Expr::Binary {
            op: BinOp::Add,
            lhs,
            rhs,
        } => {
            flatten_add_chain(lhs, out);
            flatten_add_chain(rhs, out);
        }
        Expr::Paren(inner) => flatten_add_chain(inner, out),
        _ => out.push(e),
    }
}

fn expr_is_str_literal(e: &Expr) -> bool {
    match e {
        Expr::Str(_) => true,
        Expr::Paren(inner) => expr_is_str_literal(inner),
        _ => false,
    }
}

/// 字符串要放进 `println!("...")` 的模板里，需要转义引号、反斜杠和花括号。
/// 字符串字面量转成 Rust 源码里的字面量（转义引号、反斜杠、换行等），不动花括号。
fn escape_str_literal(s: &str) -> String {
    let mut o = String::new();
    for c in s.chars() {
        match c {
            '\\' => o.push_str("\\\\"),
            '"' => o.push_str("\\\""),
            '\n' => o.push_str("\\n"),
            '\t' => o.push_str("\\t"),
            '\r' => o.push_str("\\r"),
            _ => o.push(c),
        }
    }
    o
}

/// 统计 Rust 格式串里的 `{}` 个数；`{{` / `}}` 是转义不计数。
/// 孤立的 `{` 或 `}` 返回 `None`（Rust 同样不接受）。
fn count_placeholders(fmt: &str) -> Option<usize> {
    let mut it = fmt.chars().peekable();
    let mut n = 0usize;
    while let Some(c) = it.next() {
        match c {
            '{' => match it.peek() {
                Some('{') => {
                    it.next();
                }
                Some('}') => {
                    it.next();
                    n += 1;
                }
                _ => return None,
            },
            '}' => match it.peek() {
                Some('}') => {
                    it.next();
                }
                _ => return None,
            },
            _ => {}
        }
    }
    Some(n)
}

fn escape_in_fmt(s: &str) -> String {
    escape_str_literal(s).replace('{', "{{").replace('}', "}}")
}

fn collect_mutated(stmts: &[Stmt]) -> HashSet<String> {
    let mut set = HashSet::new();
    walk_mut(stmts, &mut set);
    set
}

/// 赋值目标的最左侧变量名（`y.v` → `y`，`v[i]` → `v`；`this.x` 不返回）。
fn assign_root_ident(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Ident(n) => Some(n.clone()),
        Expr::Field(recv, _) | Expr::Index(recv, _) => assign_root_ident(recv),
        _ => None,
    }
}

fn walk_mut(stmts: &[Stmt], set: &mut HashSet<String>) {
    for s in stmts {
        match s {
            Stmt::Assign {
                target: Expr::Ident(n),
                ..
            } => {
                set.insert(n.clone());
            }
            // `obj.field = x` / `v[i] = x` 也要让根变量可变，否则克隆出来的副本无法修改。
            Stmt::Assign { target, .. } => {
                if let Some(root) = assign_root_ident(target) {
                    set.insert(root);
                }
            }
            Stmt::If { then, els, .. } => {
                walk_mut(then, set);
                if let Some(e) = els {
                    walk_mut(e, set);
                }
            }
            Stmt::While { body, .. } => walk_mut(body, set),
            Stmt::ForEach { body, .. } => walk_mut(body, set),
            Stmt::ForRange { body, .. } => walk_mut(body, set),
            Stmt::Switch {
                subject,
                cases,
                default,
                ..
            } => {
                walk_mut_expr(subject, set);
                for c in cases {
                    if let Some(g) = &c.guard {
                        walk_mut_expr(g, set);
                    }
                    walk_mut(&c.body, set);
                }
                if let Some(d) = default {
                    walk_mut(d, set);
                }
            }
            Stmt::Throw { expr, .. } => walk_mut_expr(expr, set),
            Stmt::Expr { expr, .. } => walk_mut_expr(expr, set),
            Stmt::Return { value: Some(e), .. } => walk_mut_expr(e, set),
            _ => {}
        }
    }
}

fn walk_mut_expr(e: &Expr, set: &mut HashSet<String>) {
    match e {
        Expr::Call { recv, name, args } => {
            if matches!(
                name.as_str(),
                "add" | "push" | "insert" | "remove" | "clear" | "set" | "put"
            ) {
                if let Some(r) = recv {
                    if let Expr::Ident(n) = &**r {
                        set.insert(n.clone());
                    }
                }
            }
            if let Some(r) = recv {
                walk_mut_expr(r, set);
            }
            for a in args {
                walk_mut_expr(a, set);
            }
        }
        Expr::Binary { lhs, rhs, .. } => {
            walk_mut_expr(lhs, set);
            walk_mut_expr(rhs, set);
        }
        Expr::Unary { expr, .. } => walk_mut_expr(expr, set),
        Expr::Field(r, _) => walk_mut_expr(r, set),
        Expr::Index(a, b) => {
            walk_mut_expr(a, set);
            walk_mut_expr(b, set);
        }
        Expr::Paren(i) => walk_mut_expr(i, set),
        Expr::Try(i) => walk_mut_expr(i, set),
        Expr::Closure { body, .. } => walk_mut_expr(body, set),
        Expr::New { args, .. } => {
            for a in args {
                walk_mut_expr(a, set);
            }
        }
        _ => {}
    }
}

fn stmts_use_ident(stmts: &[Stmt], want: &str) -> bool {
    stmts.iter().any(|s| stmt_uses_ident(s, want))
}

fn stmt_uses_ident(s: &Stmt, want: &str) -> bool {
    match s {
        Stmt::Let { init, .. } => expr_uses_ident(init, want),
        Stmt::Assign { target, value, .. } => {
            expr_uses_ident(target, want) || expr_uses_ident(value, want)
        }
        Stmt::Expr { expr, .. } => expr_uses_ident(expr, want),
        Stmt::Return { value, .. } => value.as_ref().map_or(false, |e| expr_uses_ident(e, want)),
        Stmt::If { cond, then, els, .. } => {
            expr_uses_ident(cond, want)
                || stmts_use_ident(then, want)
                || els.as_ref().map_or(false, |e| stmts_use_ident(e, want))
        }
        Stmt::While { cond, body, .. } => expr_uses_ident(cond, want) || stmts_use_ident(body, want),
        Stmt::ForEach {
            iter, body, name, ..
        } => {
            name == want || expr_uses_ident(iter, want) || stmts_use_ident(body, want)
        }
        Stmt::ForRange {
            start,
            end,
            body,
            name,
            ..
        } => {
            name == want
                || expr_uses_ident(start, want)
                || expr_uses_ident(end, want)
                || stmts_use_ident(body, want)
        }
        Stmt::Break { .. } | Stmt::Continue { .. } => false,
        Stmt::Switch {
            subject,
            cases,
            default,
            ..
        } => {
            expr_uses_ident(subject, want)
                || cases.iter().any(|c| {
                    c.guard.as_ref().map_or(false, |g| expr_uses_ident(g, want))
                        || stmts_use_ident(&c.body, want)
                })
                || default
                    .as_ref()
                    .map_or(false, |d| stmts_use_ident(d, want))
        }
        Stmt::Throw { expr, .. } => expr_uses_ident(expr, want),
    }
}

fn expr_uses_ident(e: &Expr, want: &str) -> bool {
    match e {
        Expr::Ident(n) => n == want,
        Expr::Field(r, _) => expr_uses_ident(r, want),
        Expr::Call { recv, args, .. } => {
            recv.as_ref().map_or(false, |r| expr_uses_ident(r, want))
                || args.iter().any(|a| expr_uses_ident(a, want))
        }
        Expr::New { args, .. } => args.iter().any(|a| expr_uses_ident(a, want)),
        Expr::NewArray { len, .. } => expr_uses_ident(len, want),
        Expr::Binary { lhs, rhs, .. } => expr_uses_ident(lhs, want) || expr_uses_ident(rhs, want),
        Expr::Unary { expr, .. } => expr_uses_ident(expr, want),
        Expr::Index(a, b) => expr_uses_ident(a, want) || expr_uses_ident(b, want),
        Expr::Paren(i) => expr_uses_ident(i, want),
        Expr::Try(i) => expr_uses_ident(i, want),
        Expr::Closure { body, .. } => expr_uses_ident(body, want),
        _ => false,
    }
}
