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
}

impl Env {
    fn new(ret: Type) -> Self {
        Env {
            vars: HashMap::new(),
            mutated: HashSet::new(),
            ret,
        }
    }
}

struct Codegen<'a> {
    prog: &'a Program,
    out: String,
    indent: usize,
    ifaces: HashSet<String>,
    classes: HashMap<String, &'a ClassDecl>,
    diags: Vec<Diag>,
    cur_line: usize,
    in_method: bool,
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
        Codegen {
            prog,
            out: String::new(),
            indent: 0,
            ifaces,
            classes,
            diags: Vec::new(),
            cur_line: 0,
            in_method: false,
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
            Type::Array(inner) => format!("Vec<{}>", self.rust_type(inner)),
            Type::Named(n) => {
                if self.ifaces.contains(n) {
                    format!("Box<dyn {}>", n)
                } else {
                    n.clone()
                }
            }
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
        self.blank();

        let prog = self.prog;
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
        self.emit(&format!("pub trait {} {{", iface.name));
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
            self.emit(&format!("fn {}({}){};", m.name, ps.join(", "), ret));
        }
        self.indent -= 1;
        self.emit("}");
        self.blank();
    }

    fn gen_class(&mut self, class: &ClassDecl) {
        self.marker(class.line);
        if class.fields.is_empty() {
            self.emit(&format!("pub struct {};", class.name));
        } else {
            self.emit(&format!("pub struct {} {{", class.name));
            self.indent += 1;
            for f in &class.fields {
                self.marker(f.line);
                let vis = if f.vis == Vis::Public { "pub " } else { "" };
                self.emit(&format!("{}{}: {},", vis, f.name, self.rust_type(&f.ty)));
            }
            self.indent -= 1;
            self.emit("}");
        }
        self.blank();

        let prog = self.prog;
        let mut iface_methods: HashSet<String> = HashSet::new();
        for iname in &class.implements {
            if let Some(iface) = prog.interfaces.iter().find(|x| &x.name == iname) {
                for m in &iface.methods {
                    iface_methods.insert(m.name.clone());
                }
            }
        }

        // 实现接口的方法 → impl Trait for Type
        for iname in &class.implements {
            let methods: Vec<&Method> = class
                .methods
                .iter()
                .filter(|m| !m.is_static && iface_methods.contains(&m.name))
                .collect();
            if methods.is_empty() {
                continue;
            }
            self.emit(&format!("impl {} for {} {{", iname, class.name));
            self.indent += 1;
            for m in methods {
                self.gen_method(m, true);
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
            self.emit(&format!("impl {} {{", class.name));
            self.indent += 1;
            if let Some(ctor) = &class.ctor {
                self.gen_ctor(ctor);
            }
            for m in inherent {
                self.gen_method(m, false);
            }
            self.indent -= 1;
            self.emit("}");
            self.blank();
        }
    }

    fn gen_main_class(&mut self, class: &ClassDecl, main: &Method) {
        if !class.fields.is_empty() || class.methods.len() > 1 {
            self.cur_line = class.line;
            self.err(
                "含 main 的类只能有 main 方法",
                "jrust MVP 把 main 当作程序入口；其它字段和方法请放进独立的类。",
            );
        }
        self.marker(main.line);
        self.emit("fn main() {");
        self.indent += 1;
        if stmts_use_ident(&main.body, "args") {
            self.emit("let args: Vec<String> = std::env::args().skip(1).collect();");
        }
        let mut env = Env::new(Type::Void);
        for p in &main.params {
            env.vars.insert(p.name.clone(), p.ty.clone());
        }
        env.mutated = collect_mutated(&main.body);
        let prev = self.in_method;
        self.in_method = false;
        self.gen_stmts(&main.body, &mut env);
        self.in_method = prev;
        self.indent -= 1;
        self.emit("}");
        self.blank();
    }

    fn gen_ctor(&mut self, ctor: &Constructor) {
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
                    if matches!(value, Expr::Ident(n) if n == field) {
                        fields.push(field.clone());
                    } else {
                        self.cur_line = ctor.line;
                        let v = self.gen_expr(value, &mut env, None);
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
        self.emit(&format!("Self {{ {} }}", fields.join(", ")));
        self.indent -= 1;
        self.emit("}");
    }

    fn gen_method(&mut self, m: &Method, in_trait_impl: bool) {
        let vis = if in_trait_impl {
            ""
        } else if m.vis == Vis::Public {
            "pub "
        } else {
            ""
        };
        let mut ps = Vec::new();
        if !m.is_static {
            ps.push("&self".to_string());
        }
        for p in &m.params {
            ps.push(format!("{}: {}", p.name, self.rust_type(&p.ty)));
        }
        let ret = if m.ret == Type::Void {
            String::new()
        } else {
            format!(" -> {}", self.rust_type(&m.ret))
        };
        self.marker(m.line);
        self.emit(&format!("{}fn {}({}){} {{", vis, m.name, ps.join(", "), ret));
        self.indent += 1;
        let mut env = Env::new(m.ret.clone());
        for p in &m.params {
            env.vars.insert(p.name.clone(), p.ty.clone());
        }
        env.mutated = collect_mutated(&m.body);
        let prev = self.in_method;
        self.in_method = !m.is_static;
        self.gen_stmts(&m.body, &mut env);
        self.in_method = prev;
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
                match init {
                    Expr::Ident(src) => {
                        if !ty.is_copy_scalar() && env.vars.contains_key(src) {
                            self.emit(&format!(
                                "// jrust 提示: `{}` 是移动语义——所有权从 `{}` 转给 `{}`，之后再用 `{}` 会被借用检查器拒绝；需要两份就写 `{}.clone()`。",
                                src, src, name, src, src
                            ));
                        }
                    }
                    Expr::Unary {
                        op: UnOp::Move,
                        expr,
                    } => {
                        if let Expr::Ident(src) = &**expr {
                            self.emit(&format!(
                                "// jrust 提示: 显式 `move` —— 所有权从 `{}` 搬到 `{}`。Rust 的赋值本身就是 move，所以产物与不加 `move` 相同；这个关键字只是把意图写出来。",
                                src, name
                            ));
                        }
                    }
                    _ => {}
                }
                let ty_str = self.rust_type(ty);
                let init_str = self.gen_expr(init, env, Some(ty));
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
                let t = self.gen_expr(target, env, None);
                let v = self.gen_expr(value, env, None);
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
                        let v = self.gen_expr(e, env, Some(&ret));
                        self.emit(&format!("return {};", v));
                    }
                    None => self.emit("return;"),
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
        }
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
            Expr::Ident(n) => n.clone(),
            Expr::This => "self".to_string(),
            Expr::Field(recv, name) => {
                let r = self.gen_expr(recv, env, None);
                format!("{}.{}", r, name)
            }
            Expr::Call { recv, name, args } => self.gen_call(recv, name, args, env),
            Expr::New { class, args } => {
                let inner = self.gen_new(class, args, env);
                if let Some(Type::Named(t)) = expect {
                    if self.ifaces.contains(t) {
                        return format!("Box::new({})", inner);
                    }
                }
                inner
            }
            Expr::Binary { op, lhs, rhs } => {
                if *op == BinOp::Add && (expr_is_str_literal(lhs) || expr_is_str_literal(rhs)) {
                    self.err(
                        "jrust 不支持用 + 拼接字符串",
                        "Rust 里拼字符串用 format!(\"{} {}\", a, b) 或 println! 的格式串；`+` 只做数值加法。",
                    );
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
                format!("{}[{}]", x, y)
            }
            Expr::Paren(inner) => {
                let x = self.gen_expr(inner, env, expect);
                format!("({})", x)
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

    fn gen_new(&mut self, class: &str, args: &[Expr], env: &mut Env) -> String {
        if matches!(class, "ArrayList" | "LinkedList" | "Vector") {
            if !args.is_empty() {
                self.err(
                    "jrust MVP 只支持无参的 new ArrayList<>()",
                    "Rust 用 `Vec::new()` 再逐个 `push`。",
                );
            }
            return "Vec::new()".to_string();
        }
        let arg_strs: Vec<String> = args.iter().map(|a| self.gen_expr(a, env, None)).collect();
        let known = self.classes.contains_key(class);
        let has_ctor = self
            .classes
            .get(class)
            .map(|c| c.ctor.is_some())
            .unwrap_or(false);
        if !known {
            return format!("{}::new({})", class, arg_strs.join(", "));
        }
        if has_ctor {
            format!("{}::new({})", class, arg_strs.join(", "))
        } else if args.is_empty() {
            class.to_string()
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
    ) -> String {
        if (name == "println" || name == "print") && recv.as_ref().map_or(true, |r| is_system_out(r)) {
            let mac = if name == "println" { "println" } else { "print" };
            return self.gen_print_macro(args, env, mac);
        }

        let recv_ty = recv.as_ref().and_then(|r| self.infer_type(r, env));
        let is_list = matches!(recv_ty, Some(Type::List(_)));
        let elem_is_trait = match &recv_ty {
            Some(Type::List(inner)) => matches!(&**inner, Type::Named(n) if self.ifaces.contains(n)),
            _ => false,
        };
        let elem_is_string = match &recv_ty {
            Some(Type::List(inner)) => matches!(&**inner, Type::String),
            _ => false,
        };

        let mut arg_strs = Vec::new();
        for a in args {
            if is_list && name == "add" {
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
                arg_strs.push(self.gen_expr(a, env, None));
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

    fn infer_type(&self, e: &Expr, env: &Env) -> Option<Type> {
        match e {
            Expr::Ident(n) => env.vars.get(n).cloned(),
            Expr::New { class, .. } => Some(Type::Named(class.clone())),
            Expr::Field(recv, f) => {
                if let Some(Type::Named(cname)) = self.infer_type(recv, env) {
                    if let Some(cd) = self.classes.get(&cname) {
                        return cd.fields.iter().find(|x| &x.name == f).map(|x| x.ty.clone());
                    }
                }
                None
            }
            _ => None,
        }
    }
}

// ---------- 自由函数 ----------

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

fn walk_mut(stmts: &[Stmt], set: &mut HashSet<String>) {
    for s in stmts {
        match s {
            Stmt::Assign {
                target: Expr::Ident(n),
                ..
            } => {
                set.insert(n.clone());
            }
            Stmt::If { then, els, .. } => {
                walk_mut(then, set);
                if let Some(e) = els {
                    walk_mut(e, set);
                }
            }
            Stmt::While { body, .. } => walk_mut(body, set),
            Stmt::ForEach { body, .. } => walk_mut(body, set),
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
        Stmt::Break { .. } | Stmt::Continue { .. } => false,
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
        Expr::Binary { lhs, rhs, .. } => expr_uses_ident(lhs, want) || expr_uses_ident(rhs, want),
        Expr::Unary { expr, .. } => expr_uses_ident(expr, want),
        Expr::Index(a, b) => expr_uses_ident(a, want) || expr_uses_ident(b, want),
        Expr::Paren(i) => expr_uses_ident(i, want),
        _ => false,
    }
}
