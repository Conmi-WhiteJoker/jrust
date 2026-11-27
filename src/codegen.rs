//! 代码生成：把 jrust 的 AST 翻译成可读的 Rust 源码。
//!
//! 生成产物刻意保持「人能读懂」：带缩进、带 `// jrust:L<行号>` 源映射、
//! 遇到语义陷阱时插一条 `// jrust 提示:` 注释。

use std::collections::{HashMap, HashSet};

use crate::analysis::{self, Origin, PMode, Recv, RetMode, Sigs};
use crate::ast::*;
use crate::diag::Diag;

/// 生成结果：产物 + 不阻断构建的警告（如 `JR4101`）。
pub struct Generated {
    pub code: String,
    pub warnings: Vec<Diag>,
}

pub fn generate(prog: &Program, src_name: &str) -> Result<Generated, Vec<Diag>> {
    generate_opts(prog, src_name, false)
}

/// 多文件模式（Stage C）：`Sigs` 由调用方对**全工程**做一次分析后传入，
/// 这样跨文件调用也能拿到对方方法的形参三态与返回来源。
/// `local_roots` 是工程里的顶层模块名：只有命中的 import 才加 `crate::`，其余按外部 crate（JR-M4）。
///
/// `types` 是**全工程合并后**的声明集合（`prog` 只含当前文件，决定「输出哪些声明」）。
/// 类型表必须看全工程：跨文件类不在 `prog` 里，而构造器形参类型、方法返回类型、
/// enum / interface / Clone 判定全都要读它 —— 只喂 `prog` 会让这些信息在跨文件时全丢
/// （v1.25 修的就是这一族）。
pub fn generate_multi_with_sigs(
    prog: &Program,
    src_name: &str,
    local_roots: &HashSet<String>,
    sigs: Sigs,
    types: &Program,
) -> Result<Generated, Vec<Diag>> {
    let mut g = Codegen::with_types(prog, types);
    g.multi_file = true;
    g.local_roots = local_roots.clone();
    g.sigs = sigs;
    g.run(src_name);
    g.into_result()
}

fn generate_opts(prog: &Program, src_name: &str, multi_file: bool) -> Result<Generated, Vec<Diag>> {
    let sigs = borrow_sigs(prog)?;
    let mut g = Codegen::new(prog);
    g.multi_file = multi_file;
    g.sigs = sigs;
    g.run(src_name);
    g.into_result()
}

/// 先跑借用分析；硬错误（如 `JR4003`）直接阻断生成。
fn borrow_sigs(prog: &Program) -> Result<Sigs, Vec<Diag>> {
    let (sigs, diags) = analysis::analyze(prog);
    if diags.is_empty() {
        Ok(sigs)
    } else {
        Err(diags)
    }
}

struct Env {
    /// 局部变量 → 类型。带作用域（阶段 A）：块退出后内层声明不再可见。
    vars: crate::types::TypeEnv,
    mutated: HashSet<String>,
    ret: Type,
    /// 当前函数是否声明了 `throws`（决定 `?` 是否合法、`return` 是否包 `Ok(...)`）。
    in_throws: bool,
    /// 当前所在类型名（用于推断 `this.字段` 的类型）。
    self_class: Option<String>,
    /// 借用形参：名字 → 有效模式（`Ref` → `&T` / `&str`，`MutRef` → `&mut T`）。
    borrowed: HashMap<String, PMode>,
    /// 当前方法是否「借出 `this` 的 String 字段」（→ `-> &str`，返回处不 `.clone()`）。
    ret_borrow: bool,
    /// 本次调用里已经被移动出去的变量（Value 实参）：再次使用报 `JR4001`。
    moved: HashSet<String>,
}

impl Env {
    fn new(ret: Type) -> Self {
        Env {
            vars: crate::types::TypeEnv::new(),
            mutated: HashSet::new(),
            ret,
            in_throws: false,
            self_class: None,
            borrowed: HashMap::new(),
            ret_borrow: false,
            moved: HashSet::new(),
        }
    }
}

/// 整个程序共用一个错误枚举：单文件模式下所有 `throws` 方法都返回
/// `Result<T, JrustError>`，跨类 `?` 不需要额外的 `From` 转换。
const ERROR_ENUM: &str = "JrustError";

/// 借用返回的渲染信息（见 `Codegen::ret_sig`）。
#[derive(Default)]
struct RetSig {
    borrow: bool,
    /// `&` 后面的类型（`str` / `[T]` / `C` …）。
    inner: String,
    /// `Some("'a")` 表示需要显式生命周期。
    lt: Option<String>,
    /// 接收者是否要写成 `&'a self`。
    self_ann: bool,
    /// 需要写成 `&'a T` 的形参下标。
    param_ann: HashSet<usize>,
}

impl RetSig {
    fn ret_type(&self) -> String {
        match &self.lt {
            Some(l) => format!(" -> &{} {}", l, self.inner),
            None => format!(" -> &{}", self.inner),
        }
    }

    /// `<'a, T: Clone>`（`'a` 必须排在最前）。
    fn generic_decl(&self, type_params: &[TypeParam]) -> String {
        let base = impl_param_decl(type_params);
        match &self.lt {
            Some(l) if base.is_empty() => format!("<{}>", l),
            Some(l) => format!("<{}, {}", l, &base[1..]),
            None => base,
        }
    }
}

struct Codegen<'a> {
    /// 当前文件：决定产物里输出哪些声明（`run` 只遍历它）。
    prog: &'a Program,
    /// 全工程声明（含当前文件）：所有**类型查询**都走这张表。
    /// 单文件模式下 `types == prog`。
    types: &'a Program,
    out: String,
    indent: usize,
    ifaces: HashSet<String>,
    iface_decls: HashMap<String, &'a InterfaceDecl>,
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
    /// 当前方法是否 `async`：`await` 只在其中合法（JR-C10 → JR3022）。
    in_async: bool,
    /// 最近一次生成的表达式是否用到了 `self`（`spawn` 的 Send + 'static 检查用）。
    /// 块内首个 `this` 出现的行：`spawn` 用它把 `JR3023` 定位到真正的 `this` 行。
    this_line: Option<usize>,
    /// `@shared` 字段里按可变处理的（仍需 `Arc<Mutex<T>>`）：类名 → 字段名（JR-C13）。
    shared_mut: HashMap<String, HashSet<String>>,
    /// 当前作用域里已经持有的共享字段锁：字段名 → guard 变量名（避免二次加锁死锁）。
    guarded: HashMap<String, String>,
    /// 多文件模式：`import a.b.C;` → `use crate::a::b::C;`（JR-S4/M3）。
    multi_file: bool,
    /// 工程里的顶层模块名：只有命中的 import 才加 `crate::`，其余按外部 crate（JR-M4）。
    local_roots: HashSet<String>,
    /// 借用推断结果：接收者 `&mut self` / 参数三态（JR-O4/O7）。
    sigs: Sigs,
    /// warning 级诊断（如 `JR4101`）：只打印，不阻断生成。
    warnings: Vec<Diag>,
    /// `spawn` 捕获副本的临时变量编号（Stage E）：保证同一个方法里多次 spawn 不撞名。
    spawn_seq: usize,
}

impl<'a> Codegen<'a> {
    fn new(prog: &'a Program) -> Self {
        Self::with_types(prog, prog)
    }

    /// `prog` 决定输出内容，`types` 提供类型查询（多文件模式下是合并后的全工程）。
    fn with_types(prog: &'a Program, types: &'a Program) -> Self {
        let mut ifaces = HashSet::new();
        let mut iface_decls = HashMap::new();
        for i in &types.interfaces {
            ifaces.insert(i.name.clone());
            iface_decls.insert(i.name.clone(), i);
        }
        let mut classes = HashMap::new();
        for c in &types.classes {
            classes.insert(c.name.clone(), c);
        }
        let mut enums = HashMap::new();
        for e in &types.enums {
            enums.insert(e.name.clone(), e);
        }
        // 错误枚举是**按文件生成**的（每个用到 throws 的文件各自产出一个 `JrustError`），
        // 所以这里必须只看当前文件；用全工程集合会让每个文件都生成一份重复枚举。
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
        for c in &types.classes {
            cloneable.insert(c.name.clone());
        }
        for e in &types.enums {
            cloneable.insert(e.name.clone());
        }
        loop {
            let snapshot = cloneable.clone();
            let mut changed = false;
            for c in &types.classes {
                let params: HashSet<String> = c.type_params.iter().map(|p| p.name.clone()).collect();
                let ok = c
                    .fields
                    .iter()
                    .filter(|f| !is_const_field(f) && !field_is_shared(f))
                    .all(|f| type_cloneable(&f.ty, &snapshot, &params));
                if !ok && cloneable.remove(&c.name) {
                    changed = true;
                }
            }
            for e in &types.enums {
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
        let mut shared_mut = HashMap::new();
        for c in &types.classes {
            let s = shared_mut_fields(c, &classes);
            if !s.is_empty() {
                shared_mut.insert(c.name.clone(), s);
            }
        }
        Codegen {
            prog,
            types,
            out: String::new(),
            indent: 0,
            ifaces,
            iface_decls,
            classes,
            enums,
            error_types,
            diags: Vec::new(),
            cur_line: 0,
            in_method: false,
            cloneable,
            generic_params: HashSet::new(),
            in_async: false,
            this_line: None,
            shared_mut,
            guarded: HashMap::new(),
            multi_file: false,
            local_roots: HashSet::new(),
            sigs: Sigs::default(),
            warnings: Vec::new(),
            spawn_seq: 0,
        }
    }

    /// 收尾：有 error 级诊断就失败，否则返回产物与 warning。
    fn into_result(self) -> Result<Generated, Vec<Diag>> {
        if self.diags.is_empty() {
            // JR-G9：产物必须能过 `rustfmt --check`，而 rustfmt 要求文件以单个 `\n` 结尾。
            // `blank()` 收尾时会留下空行（文件末尾 `}\n\n`），这里统一削掉。
            Ok(Generated {
                code: self.out.trim_end_matches('\n').to_string() + "\n",
                warnings: self.warnings,
            })
        } else {
            Err(self.diags)
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

    /// `raw { ... }`（JR-X1）：内容逐字输出，前后各留一条 `// jrust:L` 源映射标记。
    ///
    /// 逐字输出意味着**不加缩进**（改了缩进就改了多行字符串字面量的内容）。
    /// 但语句位置的 raw 产物需要符合 rustfmt 期望，因此只对「不含换行的单行 raw」
    /// 补当前缩进 —— 单行内容不可能包含多行字符串，补缩进是安全的。
    fn emit_raw(&mut self, code: &str, line: usize) {
        let end = line + code.matches('\n').count();
        self.marker(line);
        let body = code.trim_matches('\n');
        if body.contains('\n') {
            for l in body.lines() {
                self.out.push_str(l);
                self.out.push('\n');
            }
        } else {
            let t = body.trim();
            if t.is_empty() {
                self.out.push('\n');
            } else {
                self.emit(t);
            }
        }
        self.marker(end);
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

    fn warn_code(&mut self, code: &'static str, msg: impl Into<String>, hint: impl Into<String>) {
        let line = self.cur_line;
        self.warnings
            .push(Diag::warning_code(line, code, msg).with_hint(hint));
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
            Expr::Ident(n) => env.vars.lookup(n).cloned(),
            Expr::Index(base, _) => match self.infer_type(base, env) {
                Some(Type::Array(inner)) => Some(*inner),
                Some(Type::List(inner)) => Some(*inner),
                _ => None,
            },
            Expr::Field(recv, f) => {
                let base = match &**recv {
                    Expr::This(_) => env.self_class.clone().map(Type::Named),
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
                        self.clone_if_needed(s, ty, core, hint_target, env)
                    } else {
                        s
                    };
                }
                let s = self.gen_expr(init, env, Some(inner));
                let s = if copy {
                    self.clone_if_needed(s, inner, core, hint_target, env)
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
                // 借用形参派生出的位置即使目标处不要求复制，也要做落地转换（搬不走）。
                let s = if copy || self.needs_owned_landing(core, env) {
                    self.clone_if_needed(s, other, core, hint_target, env)
                } else {
                    s
                };
                // 标量加宽（阶段 0 / JR-T9）：`long m = n;` → `let m: i64 = n as i64;`。
                // 源类型只有能从表达式静态拿到时才补；拿不到就原样（交给 rustc / verify）。
                match self.scalar_source_type(core, env) {
                    Some(from) => self.coerce_scalar(s, &from, other),
                    None => s,
                }
            }
        }
    }

    /// 「搬不走的取用」：借用形参派生位置（`&[T]`）与 `list.get(i)`。
    ///
    /// `list.get(i)` 生成的是**下标访问**（`v[i]`），它产出的是 `T` 的借用位置；
    /// 在 owned 上下文（`String s = xs.get(0)`）里直接赋值会 `E0507 cannot move out of index`。
    /// 放在 `println(...)` 里之所以不炸，是因为 format 参数隐式借用。
    fn needs_owned_landing(&self, e: &Expr, env: &Env) -> bool {
        if self.is_borrowed_place(e, env) {
            return true;
        }
        matches!(
            strip_parens_expr(e),
            Expr::Call { recv: Some(_), name, .. } if name == "get"
        ) && self
            .infer_type(e, env)
            .map_or(false, |t| !t.is_copy_scalar() && !matches!(t, Type::Void))
    }

    /// 变量 / 字段是「持有者」：按值语义需要补 `.clone()`。
    fn clone_if_needed(
        &mut self,
        s: String,
        ty: &Type,
        core: &Expr,
        hint_target: Option<&str>,
        env: &Env,
    ) -> String {
        if ty.is_copy_scalar() || matches!(ty, Type::Void) {
            return s;
        }
        // 借用形参 / 借用形参派生出的位置：按目标类型做落地转换。
        if self.is_borrowed_place(core, env) {
            return self.owned_from_borrow(s, ty);
        }
        // `list.get(i)` → `v[i]`：下标不能搬出 owned 值，按类型做落地转换。
        // 注意 `.clone()` 比 `.to_string()` 更通用，`owned_from_borrow` 已按类型分流。
        if !is_holder_expr(core) {
            if matches!(
                strip_parens_expr(core),
                Expr::Call { recv: Some(_), name, .. } if name == "get"
            ) {
                return self.owned_from_borrow(s, ty);
            }
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
                if let Some(iface) = self.iface_decls.get(&c) {
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

    /// 实例方法调用的签名（形参三态 + 返回模式）；接收者类型查不到时返回 `None`。
    fn call_sig(
        &self,
        recv: &Option<Box<Expr>>,
        name: &str,
        env: &Env,
    ) -> Option<(Vec<PMode>, RetMode)> {
        if let Some(r) = recv.as_deref() {
            let (c, _) = self.infer_type(r, env).and_then(|t| self.type_binding(&t))?;
            let s = self.sigs.get(&c, name)?;
            Some((s.params.clone(), s.ret.clone()))
        } else {
            let c = env.self_class.clone()?;
            if let Some(s) = self.sigs.get(&c, name) {
                return Some((s.params.clone(), s.ret.clone()));
            }
            self.types
                .interfaces
                .iter()
                .find_map(|i| self.sigs.get(&i.name, name))
                .map(|s| (s.params.clone(), s.ret.clone()))
        }
    }

    /// 表达式是否是「借出字段 / 形参」的调用（结果为 `&T`）。
    fn call_returns_str_ref(&self, e: &Expr, env: &Env) -> bool {
        match strip_parens_expr(e) {
            Expr::Call { recv, name, .. } => self
                .call_sig(recv, name, env)
                .map_or(false, |(_, r)| r.is_borrow()),
            _ => false,
        }
    }

    /// 生成一个实参；形参是 `T?` 时把非空值包成 `Some(...)`。
    fn gen_arg(&mut self, a: &Expr, pt: &Type, env: &mut Env) -> String {
        match pt {
            Type::Nullable(inner) => {
                if matches!(strip_parens_expr(a), Expr::Null) || self.expr_is_nullable(a, env) {
                    self.gen_expr(a, env, Some(pt))
                } else {
                    let s = self.gen_expr(a, env, Some(inner));
                    let s = match self.scalar_source_type(a, env) {
                        Some(from) => self.coerce_scalar(s, &from, inner),
                        None => s,
                    };
                    format!("Some({})", s)
                }
            }
            _ => {
                let s = self.gen_expr(a, env, Some(pt));
                // 标量加宽（阶段 0 / JR-T9）：`h.twice(n)`（形参 double）→ `h.twice(n as f64)`。
                match self.scalar_source_type(a, env) {
                    Some(from) => self.coerce_scalar(s, &from, pt),
                    None => s,
                }
            }
        }
    }

    /// 按被调方法的形参三态生成实参（§13.6 调用点规则表）。
    fn gen_mode_arg(&mut self, a: &Expr, mode: PMode, pt: &Type, env: &mut Env) -> String {
        match mode {
            PMode::Value => self.gen_owned_arg(a, pt, env),
            PMode::Ref => self.gen_ref_arg(a, pt, env),
            PMode::MutRef => self.gen_mutref_arg(a, env),
        }
    }

    /// `Value` 实参：借用值补落地转换、字段补 `.clone()`、字面量补 `.to_string()`。
    fn gen_owned_arg(&mut self, a: &Expr, pt: &Type, env: &mut Env) -> String {
        match pt {
            Type::Nullable(inner) => {
                if matches!(strip_parens_expr(a), Expr::Null) || self.expr_is_nullable(a, env) {
                    return self.gen_expr(a, env, Some(pt));
                }
                return format!("Some({})", self.gen_owned_arg(a, inner, env));
            }
            _ => {}
        }
        let core = strip_parens_expr(a);
        if pt.is_copy_scalar() {
            return self.gen_arg(a, pt, env);
        }
        if self.is_borrowed_place(core, env) {
            let s = self.gen_expr(a, env, None);
            return self.owned_from_borrow(s, pt);
        }
        if let Expr::Field(..) = core {
            let s = self.gen_expr(a, env, Some(pt));
            return if self.is_cloneable_type(pt) {
                format!("{}.clone()", s)
            } else {
                s
            };
        }
        if let Expr::Ident(n) = core {
            let s = self.gen_arg(a, pt, env);
            // 生成之后再登记移动，否则本次使用会被自己误判为 use-after-move。
            let owned = env.vars.lookup(n).map_or(true, |t| !t.is_copy_scalar());
            if owned && !env.borrowed.contains_key(n) {
                env.moved.insert(n.clone());
            }
            return s;
        }
        self.gen_arg(a, pt, env)
    }

    /// `Ref` 实参：变量 / 字段加 `&`；已经是引用的形参与 `&str` 字面量原样传。
    fn gen_ref_arg(&mut self, a: &Expr, pt: &Type, env: &mut Env) -> String {
        let core = strip_parens_expr(a);
        if let Type::Nullable(inner) = pt {
            if matches!(core, Expr::Null) {
                return "&None".to_string();
            }
            if self.expr_is_nullable(core, env) {
                let s = self.gen_expr(a, env, None);
                return format!("&{}", s);
            }
            return format!("&Some({})", self.gen_owned_arg(a, inner, env));
        }
        if matches!(core, Expr::Str(_)) || self.is_borrowed_value(a, env) {
            return self.gen_expr(a, env, None);
        }
        format!("&{}", self.gen_expr(a, env, None))
    }

    /// `MutRef` 实参：变量 / 字段加 `&mut`；已经是 `&mut` 的形参原样传。
    fn gen_mutref_arg(&mut self, a: &Expr, env: &mut Env) -> String {
        if self.is_borrowed_value(a, env) {
            return self.gen_expr(a, env, None);
        }
        let s = self.gen_expr(a, env, None);
        format!("&mut {}", s)
    }

    /// `Ref` 实参（形参类型不可见）：变量 / 字段加 `&`；`&str` 字面量与借用值原样传。
    fn gen_default_ref_arg(&mut self, a: &Expr, env: &mut Env) -> String {
        let core = strip_parens_expr(a);
        if matches!(core, Expr::Str(_)) || self.is_borrowed_value(a, env) {
            return self.gen_expr(a, env, None);
        }
        format!("&{}", self.gen_expr(a, env, None))
    }

    /// 形参三态已知、但形参类型不可见（跨文件 / 外部 crate）：按模式生成实参。
    fn gen_mode_arg_unknown(&mut self, a: &Expr, mode: PMode, env: &mut Env) -> String {
        match mode {
            PMode::Value => self.gen_expr(a, env, None),
            PMode::Ref => self.gen_default_ref_arg(a, env),
            PMode::MutRef => self.gen_mutref_arg(a, env),
        }
    }

    /// 接收者是否「外部类型」（阶段 E / `Ty::Unknown` 的显式判定）。
    ///
    /// 这是 V2 类型层里 `Unknown` 概念的唯一落地点：**本工程没有源码的类型**。
    /// 语义上等价于 `Ty::Unknown`——jrust 无法判定它的方法签名、字段、trait 实现，
    /// 因此所有基于它的判断都必须降级为「放行 + warning」（JR-O6 / `JR4101`），
    /// 而不是静默猜一个答案。
    ///
    /// 三个条件是**且**关系：
    /// 1. 名字能在类型位置出现（`Named` / `Generic`）；
    /// 2. 不是本工程声明的 class / interface / enum；
    /// 3. 不是内置映射类型（`List` / `Map` / `String` 等在 parser 里已转成 `Type` 变体，
    ///    走到这里的只会是用户直接写内建名的边角情况）。
    fn is_external_recv_type(&self, t: Option<&Type>) -> bool {
        let name = match t {
            Some(Type::Named(n)) => n,
            Some(Type::Generic(n, _)) => n,
            _ => return false,
        };
        !self.classes.contains_key(name)
            && !self.ifaces.contains(name)
            && !self.enums.contains_key(name)
            && !is_known_builtin_type(name)
    }

    /// 该类字段是否是 `@shared`（JR-C13）。
    fn is_shared_field(&self, class: &str, field: &str) -> bool {
        self.classes.get(class).map_or(false, |c| {
            c.fields
                .iter()
                .any(|f| f.name == field && field_is_shared(f))
        })
    }

    /// 该 `@shared` 字段是否按可变处理（→ `Arc<Mutex<T>>`）。
    fn shared_is_mut(&self, class: &str, field: &str) -> bool {
        self.shared_mut
            .get(class)
            .map_or(false, |s| s.contains(field))
    }

    /// `@shared` 字段在产物里的类型：`Arc<T>` 或 `Arc<Mutex<T>>`。
    fn shared_rust_type(&self, class: &str, f: &Field) -> String {
        let base = self.rust_type(&f.ty);
        if self.shared_is_mut(class, &f.name) {
            format!("std::sync::Arc<std::sync::Mutex<{}>>", base)
        } else {
            format!("std::sync::Arc<{}>", base)
        }
    }

    /// `synchronized` 方法开头依次锁定类内 `@shared` 可变字段（JR-C12），返回已锁字段名。
    ///
    /// 按字段名排序，保证不同方法加锁顺序一致，避免交叉死锁。
    fn lock_shared_fields(&mut self, class: &str) -> Vec<String> {
        let mut fields: Vec<String> = self
            .shared_mut
            .get(class)
            .map(|s| s.iter().cloned().collect())
            .unwrap_or_default();
        fields.sort();
        for f in &fields {
            let g = format!("_g_{}", f);
            self.emit(&format!("let mut {} = self.{}.lock().unwrap();", g, f));
            self.guarded.insert(f.clone(), g);
        }
        fields
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
            Type::Set(inner) => format!("HashSet<{}>", self.rust_type(inner)),
            Type::Array(inner) => format!("Vec<{}>", self.rust_type(inner)),
            Type::Named(n) => {
                if self.ifaces.contains(n) {
                    format!("Box<dyn {}>", rust_ident(n))
                } else {
                    rust_ident(n)
                }
            }
            Type::Generic(n, args) => {
                let inner: Vec<String> = args.iter().map(|a| self.rust_type(a)).collect();
                if self.ifaces.contains(n) {
                    format!("Box<dyn {}<{}>>", rust_ident(n), inner.join(", "))
                } else {
                    format!("{}<{}>", rust_ident(n), inner.join(", "))
                }
            }
            Type::Nullable(inner) => format!("Option<{}>", self.rust_type(inner)),
        }
    }

    // ---------- 借用签名（JR-O4 / O9） ----------

    /// 形参的**有效**模式：Copy 标量恒 `Value`；`String` 不生成 `&mut String`（D2）。
    fn effective_mode(&self, mode: PMode, ty: &Type) -> PMode {
        if ty.is_copy_scalar() {
            return PMode::Value;
        }
        if *ty == Type::String && mode == PMode::MutRef {
            return PMode::Value;
        }
        mode
    }

    // ---------- 标量加宽转换（阶段 0 / SPEC JR-T9） ----------

    /// 标量的「加宽等级」：Java 的 `byte<short<int<long<float<double>` 简化版
    /// （jrust 无 byte/short，从 `int` 起）。`None` 表示不是可加宽的数值标量。
    ///
    /// `char` 按 Java 语义可加宽到 `int` 及以上，但 jrust 目前没有 char 参与算术的
    /// 用例，先不纳入，避免引入未验证的产物形态。
    fn scalar_rank(t: &Type) -> Option<u8> {
        match t {
            Type::Int => Some(1),
            Type::Long => Some(2),
            Type::Float => Some(3),
            Type::Double => Some(4),
            Type::RustPrim(n) => match n.as_str() {
                "i8" | "i16" | "i32" => Some(1),
                "i64" | "isize" => Some(2),
                // `i128` 比 `i64` 更宽，单列一级；否则 `i32 → i128` 会被误判成窄化。
                "i128" => Some(2),
                "u8" | "u16" | "u32" => Some(1),
                "u64" | "usize" => Some(2),
                "u128" => Some(2),
                "f32" => Some(3),
                "f64" => Some(4),
                _ => None,
            },
            _ => None,
        }
    }

    /// 二元运算的标量提升（阶段 C）：把较窄的一侧加宽到较宽的一侧。
    ///
    /// Java 规则：`int + double` → `double`，`int < long` → 比较前统一为 `long`。
    /// Rust 没有隐式提升，`i32 + f64` 直接 E0277，所以必须显式补 `as`。
    ///
    /// 返回 `None` 表示不需要 / 无法提升（两侧同级、任一非数值、或类型推不出来），
    /// 调用方按原样生成。
    ///
    /// **字面量特化**：`n + 1.5` 里 `1.5` 已是浮点字面量，不需要 `as`；
    /// 但 `n + 1` 里 `1` 是整数，在 `double` 上下文里要改成 `1.0`（Rust 字面量
    /// 类型由上下文推断，`n as f64 + 1.0` 才合法）。
    fn promote_binary(
        &self,
        l: &str,
        r: &str,
        lhs: &Expr,
        rhs: &Expr,
        env: &Env,
    ) -> Option<(String, String)> {
        let lt = self.scalar_source_type(lhs, env)?;
        let rt = self.scalar_source_type(rhs, env)?;
        let (lr, rr) = (Self::scalar_rank(&lt)?, Self::scalar_rank(&rt)?);
        if lr == rr {
            return None; // 同级，交给默认路径
        }
        // 取较宽的一侧作为提升目标。
        let (target, l_needs, r_needs) = if lr < rr {
            (rt.clone(), true, false)
        } else {
            (lt.clone(), false, true)
        };
        let l2 = if l_needs {
            self.coerce_scalar(l.to_string(), &lt, &target)
        } else {
            l.to_string()
        };
        let r2 = if r_needs {
            self.coerce_scalar(r.to_string(), &rt, &target)
        } else {
            r.to_string()
        };
        Some((l2, r2))
    }

    /// 把已生成的表达式串 `s` 从标量类型 `from` 加宽到 `to`。
    ///
    /// 只在**确实需要**时插 `as`：同级 / 窄化 / 任一非数值标量都原样返回。
    /// 窄化（`double` → `int`）不在本函数处理——那属于阶段 C 的 `assignable` 报错范围。
    fn coerce_scalar(&self, s: String, from: &Type, to: &Type) -> String {
        let (Some(a), Some(b)) = (Self::scalar_rank(from), Self::scalar_rank(to)) else {
            return s;
        };
        if a >= b {
            return s;
        }
        // 字面量直接改写比 `as` 更贴近手写 Rust：`double d = 1;` → `let d: f64 = 1.0;`。
        if let Some(lit) = self.scalar_literal_rewrite(&s, to) {
            return lit;
        }
        format!("{} as {}", s, self.rust_type(to))
    }

    /// 标量加宽的字面量特化：能靠 Rust 的字面量推断直接落地的，就不要 `as`。
    ///
    /// - 整数字面量 → 浮点目标：补 `.0`（`1` → `1.0`）。Rust 不接受 `let d: f64 = 1;`。
    /// - 整数字面量 → 整数目标：**原样返回**（Rust 字面量类型由上下文推断，`let m: i64 = 5;` 合法）。
    ///
    /// 只处理纯数字串（允许 `_` 分隔）：带负号、已含小数点、或含任何其他字符的一律不碰，
    /// 避免误改变量名与复合表达式。
    fn scalar_literal_rewrite(&self, s: &str, to: &Type) -> Option<String> {
        if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit() || b == b'_') {
            return None;
        }
        let to_float = matches!(to, Type::Double | Type::Float)
            || matches!(to, Type::RustPrim(n) if n == "f32" || n == "f64");
        if to_float {
            return Some(format!("{}.0", s));
        }
        // 整数目标：字面量本身就能被推断成目标整数类型，无需任何改写。
        Some(s.to_string())
    }

    /// 标量加宽用的源类型：比 `infer_type` 多认字面量。
    ///
    /// `infer_type` 对字面量返回 `None`（它服务的是"这个表达式的类型"这类结构性判断），
    /// 但加宽需要知道 `1` 是整数字面量、`1.5` 是浮点字面量。字面量在 Java 里
    /// 有明确默认类型（整数 → `int`，小数 → `double`），这里按该语义补齐。
    fn scalar_source_type(&self, e: &Expr, env: &Env) -> Option<Type> {
        match strip_parens_expr(e) {
            Expr::Int(_) => Some(Type::Int),
            Expr::Float(_) => Some(Type::Double),
            Expr::Char(_) => Some(Type::Char),
            Expr::Bool(_) => Some(Type::Boolean),
            // 字符串字面量在 `.jrust` 里是 `&str`，落到 `String` 目标时补 `.to_string()`。
            // 这里按 `String` 报告，让 `int n = "s";` 能报出「`String` 不能赋给 `i32`」。
            Expr::Str(_) => Some(Type::String),
            other => self.infer_type(other, env),
        }
    }

    /// 按三态渲染形参类型（D1–D3）。
    fn param_type(&self, mode: PMode, ty: &Type) -> String {
        match mode {
            PMode::Value => self.rust_type(ty),
            PMode::Ref => self.borrow_type(ty, false),
            PMode::MutRef => self.borrow_type(ty, true),
        }
    }

    fn borrow_type(&self, ty: &Type, mutable: bool) -> String {
        let p = if mutable { "&mut " } else { "&" };
        match ty {
            Type::String => "&str".to_string(),
            Type::List(inner) | Type::Array(inner) if mutable => {
                format!("&mut Vec<{}>", self.rust_type(inner))
            }
            Type::List(inner) | Type::Array(inner) => {
                format!("&[{}]", self.rust_type(inner))
            }
            Type::Map(k, v) => format!(
                "{}HashMap<{}, {}>",
                p,
                self.rust_type(k),
                self.rust_type(v)
            ),
            Type::Set(inner) => format!("{}HashSet<{}>", p, self.rust_type(inner)),
            Type::Nullable(inner) => format!("{}Option<{}>", p, self.rust_type(inner)),
            Type::Named(n) if self.ifaces.contains(n) => format!("{}dyn {}", p, n),
            Type::Generic(n, args) if self.ifaces.contains(n) => {
                let inner: Vec<String> = args.iter().map(|a| self.rust_type(a)).collect();
                format!("{}dyn {}<{}>", p, n, inner.join(", "))
            }
            other => format!("{}{}", p, self.rust_type(other)),
        }
    }

    /// 方法形参模式（查 `Sigs`；查不到按 `Value`）。
    fn param_modes(&self, ty: &str, name: &str) -> Vec<PMode> {
        self.sigs.params(ty, name).unwrap_or_default()
    }

    /// 实参表达式是否是「已经是引用」的形参（`&T` / `&str` / `&mut T`）。
    fn is_borrowed_value(&self, e: &Expr, env: &Env) -> bool {
        matches!(strip_parens_expr(e), Expr::Ident(n) if env.borrowed.contains_key(n))
    }

    /// 实参是否是「借用形参派生出的位置」（`p` / `p.f` / `p[i]` / `p.get(i)`）。
    fn is_borrowed_place(&self, e: &Expr, env: &Env) -> bool {
        match strip_parens_expr(e) {
            Expr::Ident(n) => env.borrowed.contains_key(n),
            Expr::Field(base, _) | Expr::Index(base, _) => self.is_borrowed_place(base, env),
            Expr::Call {
                recv: Some(base),
                name,
                ..
            } if name == "get" => self.is_borrowed_place(base, env),
            _ => false,
        }
    }

    /// 把借来的值转成 owned（落地转换，§13.2.2）。
    fn owned_from_borrow(&self, s: String, ty: &Type) -> String {
        match ty {
            Type::String => format!("{}.to_string()", s),
            Type::List(_) | Type::Array(_) => format!("{}.to_vec()", s),
            Type::Nullable(inner) => self.owned_from_borrow(s, inner),
            _ => format!("{}.clone()", s),
        }
    }

    /// 借用返回（`-> &T`）里 `&` 后面那截类型。
    fn borrow_inner(&self, ty: &Type) -> String {
        match ty {
            Type::String => "str".to_string(),
            Type::List(inner) | Type::Array(inner) => format!("[{}]", self.rust_type(inner)),
            Type::Map(k, v) => format!(
                "HashMap<{}, {}>",
                self.rust_type(k),
                self.rust_type(v)
            ),
            Type::Set(inner) => format!("HashSet<{}>", self.rust_type(inner)),
            Type::Named(n) if self.ifaces.contains(n) => format!("dyn {}", n),
            other => self.rust_type(other),
        }
    }

    /// 带生命周期的借用形参：`&'a str`。
    fn param_type_lt(&self, mode: PMode, ty: &Type, lt: Option<&str>) -> String {
        match (mode, lt) {
            (PMode::Ref, Some(l)) => format!("&{} {}", l, self.borrow_inner(ty)),
            _ => self.param_type(mode, ty),
        }
    }

    /// 返回值签名（B2）：是否借出、`&` 后的类型、是否需要显式 `'a`、哪些输入标注 `'a`。
    fn ret_sig(
        &self,
        ty: &str,
        name: &str,
        params: &[Param],
        modes: &[PMode],
        recv_mut: bool,
    ) -> RetSig {
        let RetMode::Borrow(srcs) = self.sigs.ret(ty, name) else {
            return RetSig::default();
        };
        let inner = srcs
            .iter()
            .find_map(|o| match o {
                Origin::SelfRef => Some("str".to_string()),
                Origin::Param(i) => params.get(*i).map(|p| self.borrow_inner(&p.ty)),
            })
            .unwrap_or_else(|| "str".to_string());

        // 借用输入 = `&self` / `&mut self` + 所有非 Value 形参（含 `&mut`）。
        let self_input = true;
        let ref_params: Vec<usize> = (0..params.len())
            .filter(|i| {
                let mode = self.effective_mode(
                    modes.get(*i).copied().unwrap_or(PMode::Value),
                    &params[*i].ty,
                );
                !mode.is_value()
            })
            .collect();
        let inputs = ref_params.len() + usize::from(self_input);

        // 生命周期省略：唯一的借用输入恰好就是唯一的来源。
        let elide = inputs == 1
            && srcs.len() == 1
            && match srcs[0] {
                Origin::SelfRef => self_input && !recv_mut,
                Origin::Param(i) => !self_input && ref_params == vec![i],
            };
        let lt = if elide { None } else { Some("'a".to_string()) };
        let self_ann = !elide && srcs.contains(&Origin::SelfRef);
        let param_ann: HashSet<usize> = if elide {
            HashSet::new()
        } else {
            srcs.iter()
                .filter_map(|o| match o {
                    Origin::Param(i) => Some(*i),
                    Origin::SelfRef => None,
                })
                .collect()
        };
        RetSig {
            borrow: true,
            inner,
            lt,
            self_ann,
            param_ann,
        }
    }

    // ---------- 顶层 ----------

    /// 类型名撞上「不能用 `r#` 挽救」的关键字时直接报错（JR-L1）。
    ///
    /// 名称解析（阶段 B / JR6001）：类型位置出现的名字必须在符号表里。
    ///
    /// 覆盖两类此前静默放行的写法：
    /// - `var x = 1;` —— `var` 不是类型（SPEC JR-T7 明确不支持）；
    /// - `Intger x = 1;` —— 拼错的类型名。
    ///
    /// 两者都走 parser 的 `Tok::Ident → Type::Named` 兜底，此前一路透传到 rustc。
    ///
    /// **误报控制**（这条检查最容易误伤合法程序，逐条说明为什么安全）：
    /// 1. 只查 `Type::Named` —— 标量（`int` 等）是关键字走 `Kw`，不进 `Named`；
    ///    `List` / `Map` / `String` 等内建名在 parser 里已转成对应 `Type` 变体，也不会是 `Named`。
    /// 2. 泛型参数单独收集（类级 + 方法级 + 接口级），不参与"未知类型"判定。
    /// 3. 外部 crate 的类型：只要 `import` 里出现过该名字（或它的末段），就放行——
    ///    jrust 看不到外部源码，不能武断判错（与 JR-O6 同一原则）。
    /// 4. `raw` 块内完全不检查（JR-X3：raw 不参与 jrust 诊断）。
    fn check_unknown_type_names(&mut self) {
        let st = crate::types::SymbolTable::build(self.types);
        // 外部 import 的末段名字：`import serde_json::Value;` → `Value`。
        let external: HashSet<String> = self
            .types
            .imports
            .iter()
            .filter(|i| !i.starts_with("java."))
            .map(|i| i.rsplit(['.', ':']).next().unwrap_or(i).to_string())
            .collect();

        let mut bad: Vec<(usize, String)> = Vec::new();

        // 收集所有「合法类型名」：符号表里的 + 泛型参数。
        let mut known: HashSet<String> = HashSet::new();
        for c in &self.types.classes {
            known.insert(c.name.clone());
            for p in &c.type_params {
                known.insert(p.name.clone());
            }
            for m in &c.methods {
                for p in &m.type_params {
                    known.insert(p.name.clone());
                }
            }
        }
        for i in &self.types.interfaces {
            known.insert(i.name.clone());
            for p in &i.type_params {
                known.insert(p.name.clone());
            }
            for m in &i.methods {
                for p in &m.type_params {
                    known.insert(p.name.clone());
                }
            }
        }
        for e in &self.types.enums {
            known.insert(e.name.clone());
            for p in &e.type_params {
                known.insert(p.name.clone());
            }
        }
        let _ = &st; // 符号表用于后续阶段（JR6003/JR6004），此处只用 known 集合。

        // Java 包装类型名：parser 已把它们映射成标量，不会出现在 `Type::Named` 里，
        // 但它们必须进 `known`——否则 `Intger x = 1;` 无法被判为「Integer 的拼错」，
        // 只能降级成 warning，失去了这条检查最有价值的场景。
        for n in [
            "Integer", "Long", "Double", "Float", "Boolean", "Character", "String", "Object",
            "List", "ArrayList", "LinkedList", "Collection", "Vec", "Map", "HashMap", "Set",
            "HashSet",
        ] {
            known.insert(n.to_string());
        }

        // 逐处扫描类型位置。
        let probe = |ty: &Type, line: usize, bad: &mut Vec<(usize, String)>| {
            if let Some(n) = unknown_named_in(ty, &known, &external) {
                bad.push((line, n));
            }
        };

        for c in &self.prog.classes {
            for f in &c.fields {
                probe(&f.ty, f.line, &mut bad);
            }
            if let Some(ctor) = &c.ctor {
                for p in &ctor.params {
                    probe(&p.ty, p.line, &mut bad);
                }
            }
            for m in &c.methods {
                probe(&m.ret, m.line, &mut bad);
                for p in &m.params {
                    probe(&p.ty, p.line, &mut bad);
                }
            }
        }
        for i in &self.prog.interfaces {
            for m in &i.methods {
                probe(&m.ret, m.line, &mut bad);
                for p in &m.params {
                    probe(&p.ty, p.line, &mut bad);
                }
            }
        }

        // 局部变量声明：`var x = 1;` / `Intger x = 1;` 都在这里。
        for c in &self.prog.classes {
            for m in &c.methods {
                collect_local_types(&m.body, &mut bad, &known, &external);
            }
            if let Some(ctor) = &c.ctor {
                collect_local_types(&ctor.body, &mut bad, &known, &external);
            }
        }

        bad.sort();
        bad.dedup();
        for (line, name) in bad {
            self.cur_line = line;
            if name == "var" {
                // `var` 能确定是错的（SPEC JR-T7 明确不支持），报 error。
                self.err_code(
                    "JR6001",
                    "`var` 不是 jrust 的类型：声明处必须写全类型",
                    "写具体类型，例如 `int x = 1;`；`var` 的类型推断不在 jrust 支持范围内（SPEC JR-T7）。",
                );
            } else if let Some(similar) = closest_known(&name, &known) {
                // 能找到拼写相近的已声明类型 → 基本可以断定是拼错，报 error。
                self.err_code(
                    "JR6001",
                    format!("未知类型 `{}`（是不是 `{}`？）", name, similar),
                    format!(
                        "检查拼写：工程里声明的是 `{}`。若确实是另一个类型，先声明 `class {} {{ ... }}`。",
                        similar, name
                    ),
                );
            } else {
                // 找不到相近类型 → 可能是外部 crate 的类型（jrust 看不到源码）。
                // 按 JR-O6「无法判定时给 warning 而非武断判错」处理，不阻断生成。
                self.warn_code(
                    "JR6102",
                    format!("类型 `{}` 在本工程里找不到声明", name),
                    format!(
                        "若是外部 crate 的类型，补 `import` 并在 Cargo.toml 声明依赖；若是拼写错误，改成已声明的类型名；若是自建类型，补上 `class {} {{ ... }}`。",
                        name
                    ),
                );
            }
        }
    }

    /// 未知方法（阶段 B / JR6004）。
    ///
    /// **只对能确定的场景报错**，因为方法名解析的误报面比类型名大得多：
    ///
    /// 报错的三个必要条件（全部满足才报）：
    /// 1. 接收者的静态类型能在本工程里解析到（`infer_type` 拿到 `Named`/`Generic`）；
    /// 2. 该类型**有源码**（在 `classes` 或 `iface_decls` 里）——外部 crate 类型一律放行；
    /// 3. 该类型（含其实现的接口、父接口）的方法表里确实没有这个名字。
    ///
    /// 不报错的情况（逐条说明为什么）：
    /// - 接收者是内置类型（`String` / `List` / `Map` / `Set` / 标量）：Java 方法名与 Rust
    ///   不同名（`substring` / `charAt` / `toUpperCase` 在 Rust 里不存在），jrust 目前是
    ///   **原样透传**，交给 `jrust verify` 暴露。这类留给阶段 D 的「内置方法白名单」。
    /// - 接收者类型推不出来（`infer_type` 返回 `None`）：无法判定，放行。
    /// - 接收者是泛型参数 `T`：上界方法集在阶段 E 才完整，先放行。
    /// - `raw` 块内的调用：JR-X3 规定 raw 不参与 jrust 诊断。
    fn check_unknown_methods(&mut self) {
        let mut bad: Vec<(usize, String, String)> = Vec::new();

        for c in &self.prog.classes {
            // 收集本工程全部「已知方法名」的来源：本类 + 实现的接口闭包 + 所有类
            // （jrust 的调用点推断是按名字并集工作的，见 `Sigs::mut_names`）。
            let mut body_env = Env::new(c.methods.first().map_or(Type::Void, |m| m.ret.clone()));
            body_env.self_class = Some(c.name.clone());
            for m in &c.methods {
                for p in &m.params {
                    body_env.vars.declare(p.name.clone(), p.ty.clone());
                }
            }
            for m in &c.methods {
                self.collect_bad_methods(&m.body, &mut body_env, c, &mut bad);
            }
            if let Some(ctor) = &c.ctor {
                let mut cenv = Env::new(Type::Void);
                cenv.self_class = Some(c.name.clone());
                for p in &ctor.params {
                    cenv.vars.declare(p.name.clone(), p.ty.clone());
                }
                self.collect_bad_methods(&ctor.body, &mut cenv, c, &mut bad);
            }
        }

        bad.sort();
        bad.dedup();
        for (line, owner, name) in bad {
            self.cur_line = line;
            let hint = if let Some(kind) = builtin_kind_name(&owner) {
                // 内置类型：列出支持的方法，并给出 raw 的逃生口。
                format!(
                    "`{}` 只支持 {}；需要别的行为用 `raw {{ }}` 直接写 Rust。",
                    kind,
                    builtin_method_list(kind)
                )
            } else if let Some(similar) =
                closest_method(&name, &owner, &self.classes, &self.iface_decls)
            {
                format!("`{}` 里有没有 `{}`？检查拼写。", owner, similar)
            } else {
                format!("`{}` 里没有这个方法；检查拼写，或先在类里声明它。", owner)
            };
            self.err_code(
                "JR6004",
                format!("类型 `{}` 没有方法 `{}`", owner, name),
                hint,
            );
        }
    }

    /// 递归扫描语句，找出「接收者类型可确定但方法不存在」的调用。
    fn collect_bad_methods(
        &self,
        stmts: &[Stmt],
        env: &mut Env,
        cur_class: &ClassDecl,
        bad: &mut Vec<(usize, String, String)>,
    ) {
        for s in stmts {
            match s {
                Stmt::Raw { .. } => {}
                Stmt::Let { ty, name, init, line } => {
                    // 先查初始化器（此时该变量还不可见），再声明它，
                    // 这样后续语句里的 `h.calcc()` 能解析出 `h` 的类型。
                    self.probe_method_call(init, env, cur_class, *line, bad);
                    env.vars.declare(name.clone(), ty.clone());
                }
                Stmt::Expr { expr, line } => {
                    self.probe_method_call(expr, env, cur_class, *line, bad)
                }
                Stmt::Return { value: Some(e), line } => {
                    self.probe_method_call(e, env, cur_class, *line, bad)
                }
                Stmt::Assign { value, line, .. } => {
                    self.probe_method_call(value, env, cur_class, *line, bad)
                }
                Stmt::Throw { expr, line } => {
                    self.probe_method_call(expr, env, cur_class, *line, bad)
                }
                Stmt::If { cond, then, els, .. } => {
                    self.probe_method_call(cond, env, cur_class, cur_class.line, bad);
                    self.collect_bad_methods(then, env, cur_class, bad);
                    if let Some(e) = els {
                        self.collect_bad_methods(e, env, cur_class, bad);
                    }
                }
                Stmt::While { cond, body, .. } => {
                    self.probe_method_call(cond, env, cur_class, cur_class.line, bad);
                    self.collect_bad_methods(body, env, cur_class, bad);
                }
                Stmt::ForEach { iter, body, .. } => {
                    self.probe_method_call(iter, env, cur_class, cur_class.line, bad);
                    self.collect_bad_methods(body, env, cur_class, bad);
                }
                Stmt::ForRange { start, end, body, .. } => {
                    self.probe_method_call(start, env, cur_class, cur_class.line, bad);
                    self.probe_method_call(end, env, cur_class, cur_class.line, bad);
                    self.collect_bad_methods(body, env, cur_class, bad);
                }
                Stmt::Sync { body, .. } | Stmt::Spawn { body, .. } => {
                    self.collect_bad_methods(body, env, cur_class, bad)
                }
                _ => {}
            }
        }
    }

    /// 检查单个表达式里的方法调用（含嵌套子表达式）。
    fn probe_method_call(
        &self,
        e: &Expr,
        env: &mut Env,
        cur_class: &ClassDecl,
        line: usize,
        bad: &mut Vec<(usize, String, String)>,
    ) {
        match e {
            Expr::Call { recv, name, args, .. } => {
                for a in args {
                    self.probe_method_call(a, env, cur_class, line, bad);
                }
                if let Some(r) = recv {
                    self.probe_method_call(r, env, cur_class, line, bad);
                    // 静态调用 `Cls.method()`：接收者是类型名，不是值。
                    if let Expr::Ident(en) = &**r {
                        if self.classes.contains_key(en) || self.iface_decls.contains_key(en) {
                            return;
                        }
                    }
                    // 只处理「接收者类型能解析到本工程源码」的情况。
                    // 内置类型（String / List / Map / Set / 数组）走白名单检查（阶段 D）。
                    if let Some(rt) = self.infer_type(r, env) {
                        if let Some(kind) = builtin_kind(&rt) {
                            if !builtin_method_known(kind, name) {
                                bad.push((line, kind.to_string(), name.clone()));
                            }
                            return;
                        }
                    }
                    let owner = match self.infer_type(r, env) {
                        Some(t) => match type_name(&t) {
                            "" => return,
                            n => n.to_string(),
                        },
                        None => return,
                    };
                    if !self.classes.contains_key(&owner) && !self.iface_decls.contains_key(&owner) {
                        return; // 外部 crate → 放行
                    }
                    // 泛型类的方法名也按名字查（jrust 全仓按名字并集推断）。
                    if self.method_exists(&owner, name) {
                        return;
                    }
                    bad.push((line, owner, name.clone()));
                }
            }
            Expr::Binary { lhs, rhs, .. } => {
                self.probe_method_call(lhs, env, cur_class, line, bad);
                self.probe_method_call(rhs, env, cur_class, line, bad);
            }
            Expr::Unary { expr, .. } => self.probe_method_call(expr, env, cur_class, line, bad),
            Expr::Paren(inner) | Expr::Try(inner) | Expr::Await(inner) => {
                self.probe_method_call(inner, env, cur_class, line, bad)
            }
            Expr::Field(base, _) => self.probe_method_call(base, env, cur_class, line, bad),
            Expr::Index(base, idx) => {
                self.probe_method_call(base, env, cur_class, line, bad);
                self.probe_method_call(idx, env, cur_class, line, bad);
            }
            Expr::New { args, .. } => {
                for a in args {
                    self.probe_method_call(a, env, cur_class, line, bad);
                }
            }
            _ => {}
        }
    }

    /// 某类型（含其实现的接口与父接口）是否声明了该方法。
    fn method_exists(&self, owner: &str, name: &str) -> bool {
        if let Some(cd) = self.classes.get(owner) {
            if cd.methods.iter().any(|m| m.name == name) {
                return true;
            }
            for imp in &cd.implements {
                if self.iface_has_method(&type_name(imp), name, &mut HashSet::new()) {
                    return true;
                }
            }
        }
        self.iface_has_method(owner, name, &mut HashSet::new())
    }

    /// 接口（含 `extends` 链）是否声明了该方法。
    fn iface_has_method(&self, iface: &str, name: &str, seen: &mut HashSet<String>) -> bool {
        if !seen.insert(iface.to_string()) {
            return false;
        }
        let Some(d) = self.iface_decls.get(iface) else {
            return false;
        };
        if d.methods.iter().any(|m| m.name == name) {
            return true;
        }
        d.extends
            .iter()
            .any(|t| self.iface_has_method(type_name(t), name, seen))
    }

    /// 赋值 / 返回的类型兼容性（阶段 C / JR6007 / JR6008）。
    ///
    /// 用 `assignable` 判定「源类型能否赋给目标类型」；不兼容时报错。
    /// 覆盖此前全部静默透传的写法：
    /// ```java
    /// int n = "s";        // JR6007
    /// String s = 1;       // JR6007
    /// boolean b = 1;      // JR6007
    /// int f() { return "s"; }   // JR6008
    /// ```
    ///
    /// **误报控制**（比 JR6001 更保守，因为赋值是最常见的语句）：
    /// - 源类型或目标类型任一侧推不出来 → 放行（`Unknown` 语义）；
    /// - `T?` 相关一律放行，交给既有的 JR-T1 判空逻辑；
    /// - 数值之间只要满足「同级或加宽」就放行（加宽由阶段 0 的 `coerce_scalar` 补 `as`）；
    ///   窄化（`double` → `int`）报错——Java 也不允许隐式窄化；
    /// - 接口 / 类之间的赋值：只要源类型实现了目标接口就放行（含 `extends` 链）；
    /// - 泛型参数（`T`）与 `Unknown` 一律放行。
    fn check_assign_compat(&mut self) {
        let mut bad: Vec<(usize, String, String, &'static str)> = Vec::new();

        for c in &self.prog.classes {
            for m in &c.methods {
                let mut env = Env::new(m.ret.clone());
                env.self_class = Some(c.name.clone());
                for p in &m.params {
                    env.vars.declare(p.name.clone(), p.ty.clone());
                }
                collect_assign_mismatches(&m.body, &mut env, self, &mut bad);
            }
            if let Some(ctor) = &c.ctor {
                let mut env = Env::new(Type::Void);
                env.self_class = Some(c.name.clone());
                for p in &ctor.params {
                    env.vars.declare(p.name.clone(), p.ty.clone());
                }
                collect_assign_mismatches(&ctor.body, &mut env, self, &mut bad);
            }
        }

        bad.sort();
        bad.dedup();
        for (line, from, to, kind) in bad {
            self.cur_line = line;
            let code = if kind == "return" { "JR6008" } else { "JR6007" };
            let what = if kind == "return" { "返回值" } else { "赋值" };
            self.err_code(
                code,
                format!("{}的类型不匹配：`{}` 不能赋给 `{}`", what, from, to),
                "Java 只允许「同级或加宽」的隐式转换；窄化请显式转换，类型不同请改用对应类型或补转换方法。",
            );
        }
    }

    /// `assignable(from, to)`（阶段 C / §14.4 的八条规则）。
    ///
    /// 返回 `true` 表示可以直接赋值（含需要补 `as` 的加宽）。
    fn assignable(&self, from: &Type, to: &Type) -> bool {
        // 规则 1：完全相同。
        if from == to {
            return true;
        }
        // 规则 1b：`bool` 的两种表示等价（`Type::Boolean` vs `Type::RustPrim("bool")`）。
        if self.rust_type(from) == self.rust_type(to) {
            return true;
        }
        // 规则 1c：集合 / 数组不检查元素类型。`List<String> ys = xs.stream().map(...).collectList()`
        // 是合法写法（元素类型由 `map` 决定，而 jrust 的 collectList 仍按基座类型推断——
        // README 已列为已知局限）。对集合做元素级检查只会误报。
        if matches!(
            from,
            Type::List(_) | Type::Map(..) | Type::Set(_) | Type::Array(_)
        ) || matches!(
            to,
            Type::List(_) | Type::Map(..) | Type::Set(_) | Type::Array(_)
        ) {
            return true;
        }
        // 规则 2：`T?` 一律放行（JR-T1 已有专门逻辑与诊断）。
        if matches!(from, Type::Nullable(_)) || matches!(to, Type::Nullable(_)) {
            return true;
        }
        // 规则 3：数值标量——同级或加宽允许，窄化不允许。
        if let (Some(a), Some(b)) = (Self::scalar_rank(from), Self::scalar_rank(to)) {
            return a <= b;
        }
        // 规则 4：`char` ↔ 数值。Java 允许 `char` → `int`，jrust 也放行（产物补 `as`）。
        if matches!(from, Type::Char) && Self::scalar_rank(to).is_some() {
            return true;
        }
        // 规则 5：类 → 接口（含 `extends` 链）与接口 → 接口（父接口）。
        if let (Some(f), Some(t)) = (type_name_opt(from), type_name_opt(to)) {
            if self.iface_decls.contains_key(t) {
                // 目标接口：源类实现了它（或源接口继承了它）就允许。
                if let Some(cd) = self.classes.get(f) {
                    let mut seen = HashSet::new();
                    if cd
                        .implements
                        .iter()
                        .any(|imp| self.iface_closure_has(type_name(imp), &t, &mut seen))
                    {
                        return true;
                    }
                }
                let mut seen = HashSet::new();
                if self.iface_closure_has(&f, &t, &mut seen) {
                    return true;
                }
            }
        }
        // 规则 5b：`Named("Box")` 与 `Generic("Box", args)` 是同一类型的两种写法
        // （`Box b = new Box<String>(...)` 里目标侧没写实参）。按名字相同即兼容。
        if let (Some(f), Some(t)) = (type_name_opt(from), type_name_opt(to)) {
            if f == t {
                return true;
            }
        }
        // 规则 6：泛型参数 / 外部未知类型放行（无法判定不猜）。
        if matches!(from, Type::Named(n) if is_generic_param(n))
            || matches!(to, Type::Named(n) if is_generic_param(n))
        {
            return true;
        }
        // 规则 7：源或目标是本工程没有源码的类型（外部 crate）→ 放行。
        for t in [from, to] {
            if let Some(n) = type_name_opt(t) {
                if !self.classes.contains_key(n) && !self.iface_decls.contains_key(n) {
                    return true;
                }
            }
        }
        // 规则 8：其余不允许。
        false
    }

    /// 接口 `start` 是否等于 `target` 或在它的 `extends` 闭包里。
    fn iface_closure_has(&self, start: &str, target: &str, seen: &mut HashSet<String>) -> bool {
        if start == target {
            return true;
        }
        if !seen.insert(start.to_string()) {
            return false;
        }
        let Some(d) = self.iface_decls.get(start) else {
            return false;
        };
        d.extends
            .iter()
            .any(|p| self.iface_closure_has(type_name(p), target, seen))
    }

    /// 条件表达式必须是 `bool`（阶段 B / JR6009）。
    ///
    /// Java 与 Rust 在这一点上一致——`if (1)` 在两边都非法，所以这条检查
    /// **不会拒绝任何合法的 Java 写法**，只把原本推迟到 rustc 的 E0308 提前到 `check`。
    ///
    /// 只检查能确定类型的情况：`infer_type` 拿不到类型（外部 crate 调用结果、
    /// 泛型参数等）时放行。
    fn check_non_bool_conditions(&mut self) {
        let mut bad: Vec<(usize, String)> = Vec::new();

        for c in &self.prog.classes {
            for m in &c.methods {
                let mut menv = Env::new(m.ret.clone());
                menv.self_class = Some(c.name.clone());
                for p in &m.params {
                    menv.vars.declare(p.name.clone(), p.ty.clone());
                }
                collect_non_bool_conds(&m.body, &mut menv, self, &mut bad);
            }
            if let Some(ctor) = &c.ctor {
                let mut cenv = Env::new(Type::Void);
                cenv.self_class = Some(c.name.clone());
                for p in &ctor.params {
                    cenv.vars.declare(p.name.clone(), p.ty.clone());
                }
                collect_non_bool_conds(&ctor.body, &mut cenv, self, &mut bad);
            }
        }

        bad.sort();
        bad.dedup();
        for (line, tn) in bad {
            self.cur_line = line;
            self.err_code(
                "JR6009",
                format!("条件表达式的类型是 `{}`，不是 `boolean`", tn),
                "`if` / `while` 的条件必须是布尔值；比较写成 `n != 0` / `flag`，不要直接写数值。",
            );
        }
    }

    /// `r#crate` / `r#Self` / `r#self` / `r#super` 本身非法，所以这几个只能让用户改名；
    /// 其余关键字由 `rust_ident` 加 `r#` 透明处理，不打扰用户。
    fn check_type_name_conflicts(&mut self) {
        let prog = self.prog;
        let mut bad: Vec<(usize, String)> = Vec::new();
        for c in &prog.classes {
            if keyword_conflict(&c.name) {
                bad.push((c.line, c.name.clone()));
            }
        }
        for i in &prog.interfaces {
            if keyword_conflict(&i.name) {
                bad.push((i.line, i.name.clone()));
            }
        }
        for e in &prog.enums {
            if keyword_conflict(&e.name) {
                bad.push((e.line, e.name.clone()));
            }
        }
        for (line, name) in bad {
            self.err_code(
                "JR3030",
                format!("`{}` 不能作为类型名：它是 Rust 关键字且无法用 `r#` 转义", name),
                "改一个名字，例如 `Crate` / `Myself` / `Super`。",
            );
            let _ = line;
        }
    }

    fn run(&mut self, src_name: &str) {
        self.emit(&format!(
            "// 由 jrust 从 {} 生成。请对照 .jrust 源码阅读，不要直接改这个文件。",
            src_name
        ));
        self.emit("// 每段代码前的 `// jrust:L<行号>` 指向 .jrust 源码行。");
        // `non_snake_case`：jrust 沿用 Java 驼峰命名，生成的 `addTwice` / `getTag` 必然触发；
        // `unused_assignments`：`T x = a; x = b;` 这种 jrust 写法（先声明后赋值）在产物里
        // 表现为「赋值后未读」。两者都是**生成形态**带来的噪声，不是用户 bug，必须 allow ——
        // 否则 JR-G9 的「`-D warnings` 下可编译」对任何驼峰项目都不成立。
        self.emit(
            "#![allow(dead_code, unused_variables, unused_mut, unused_imports, unused_parens, non_snake_case, unused_assignments)]",
        );
        self.check_type_name_conflicts();
        self.check_unknown_type_names();
        self.check_unknown_methods();
        self.check_non_bool_conditions();
        self.check_assign_compat();
        // `java.*` 的 import 只需让 jrust 认识类型名，不生成代码；
        // 其余路径当作 Rust 的 use 原样输出（圆点转 `::`），例如 import std.vec.Vec;
        let imports = self.prog.imports.clone();
        let mut seen = std::collections::HashSet::new();
        for imp in &imports {
            if imp.starts_with("java.") || !seen.insert(imp.clone()) {
                continue;
            }
            // 每个段都要按标识符渲染：`import box.thing.Thing` → `use r#box::thing::Thing;`
            // 否则撞 Rust 关键字会直接生成非法 Rust（JR-L1）。
            let path = imp
                .split('.')
                .map(rust_ident)
                .collect::<Vec<_>>()
                .join("::");
            // 多文件模式（JR-S4/M3）：跨模块引用要带上 crate 根，写成 `use crate::a::b::C;`。
            // `std` / `core` / `alloc` 是 Rust 自带的外部 crate，`crate` / `self` / `super`
            // 本身就是根路径，都不加前缀。
            let first = imp.split('.').next().unwrap_or("");
            let path = if self.multi_file
                && self.local_roots.contains(first)
                && !matches!(first, "std" | "core" | "alloc" | "crate" | "self" | "super")
            {
                format!("crate::{}", path)
            } else {
                path
            };
            self.emit(&format!("use {};", path));
        }
        // Map / Set 统一映射到 std；未用到时由 allow(unused_imports) 兜住。
        if !seen.contains("std.collections.HashMap") {
            self.emit("use std::collections::HashMap;");
        }
        if !seen.contains("std.collections.HashSet") {
            self.emit("use std::collections::HashSet;");
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
            if class.is_exception {
                self.gen_exception(class);
            } else if !has_main {
                self.gen_class(class);
            }
        }
        for class in &prog.classes {
            if let Some(main) = class.methods.iter().find(|m| m.is_static && m.name == "main") {
                self.gen_main_class(class, main);
            }
        }
        // 类内 `raw { ... }` 成员按顶层条目输出（JR-X1）。
        for class in &prog.classes {
            for r in &class.raw_members {
                self.emit_raw(&r.code, r.line);
                self.blank();
            }
        }
    }

    fn gen_trait(&mut self, iface: &InterfaceDecl) {
        self.marker(iface.line);
        self.emit_docs(&iface.docs);
        for l in self.attr_lines(&iface.attrs) {
            self.emit(&l);
        }
        // `interface A extends B, C` → `trait A: B + C`（JR-D6）。
        let supers = if iface.extends.is_empty() {
            String::new()
        } else {
            let parts: Vec<String> = iface.extends.iter().map(|t| self.trait_ref_type(t)).collect();
            format!(": {}", parts.join(" + "))
        };
        self.emit(&format!(
            "pub trait {}{}{} {{",
            rust_ident(&iface.name),
            type_param_decl(&iface.type_params),
            supers
        ));
        self.indent += 1;
        for m in &iface.methods {
            // static 方法单独转成自由函数（见下面的循环）。
            if m.is_static {
                continue;
            }
            self.marker(m.line);
            self.emit_docs(&m.docs);
            for l in self.attr_lines(&m.attrs) {
                self.emit(&l);
            }
            let recv_mut = self.sigs.recv(&iface.name, &m.name) == Recv::Mut;
            let mutated = m.body.as_ref().map(|b| self.collect_mutated_with_sigs(b));
            let modes = self.param_modes(&iface.name, &m.name);
            let rs = self.ret_sig(&iface.name, &m.name, &m.params, &modes, recv_mut);
            let mut ps = vec![match (recv_mut, rs.self_ann) {
                (true, _) => "&mut self".to_string(),
                (false, true) => "&'a self".to_string(),
                (false, false) => "&self".to_string(),
            }];
            for (i, p) in m.params.iter().enumerate() {
                // 只有默认方法才在形参上写 `mut`；纯声明不涉及方法体。
                let mut_kw = if mutated.as_ref().map_or(false, |s| s.contains(&p.name)) {
                    "mut "
                } else {
                    ""
                };
                let raw = modes.get(i).copied().unwrap_or(PMode::Value);
                let mode = self.effective_mode(raw, &p.ty);
                let lt = if rs.param_ann.contains(&i) {
                    rs.lt.as_deref()
                } else {
                    None
                };
                ps.push(format!(
                    "{}{}: {}",
                    mut_kw,
                    p.name,
                    self.param_type_lt(mode, &p.ty, lt)
                ));
            }
            let ret_borrow = rs.borrow;
            let ret = if ret_borrow {
                rs.ret_type()
            } else if m.ret == Type::Void {
                String::new()
            } else {
                format!(" -> {}", self.rust_type(&m.ret))
            };
            let sig = format!(
                "fn {}{}({}){}",
                m.name,
                rs.generic_decl(&m.type_params),
                ps.join(", "),
                ret
            );
            match &m.body {
                // 默认方法：trait 内直接给实现（JR-D10）。
                Some(body) => {
                    self.emit(&format!("{} {{", sig));
                    self.indent += 1;
                    let prev_generic = self.generic_params.clone();
                    self.generic_params
                        .extend(m.type_params.iter().map(|p| p.name.clone()));
                    let mut env = Env::new(m.ret.clone());
                    for (i, p) in m.params.iter().enumerate() {
                        env.vars.declare(p.name.clone(), p.ty.clone());
                        let raw = modes.get(i).copied().unwrap_or(PMode::Value);
                        let mode = self.effective_mode(raw, &p.ty);
                        if !mode.is_value() {
                            env.borrowed.insert(p.name.clone(), mode);
                        }
                    }
                    env.mutated = mutated.clone().unwrap_or_default();
                    env.self_class = Some(iface.name.clone());
                    env.ret_borrow = ret_borrow;
                    let prev = self.in_method;
                    self.in_method = !m.is_static;
                    self.gen_stmts(body, &mut env);
                    self.in_method = prev;
                    self.generic_params = prev_generic;
                    self.indent -= 1;
                    self.emit("}");
                }
                None => self.emit(&format!("{};", sig)),
            }
        }
        self.indent -= 1;
        self.emit("}");
        self.blank();

        // 接口 static 方法 → 顶层自由函数 `Iface_method(...)`（JR-D10）。
        // Rust 不允许 `Trait::assoc_fn()`（E0790，Self 不明确），因此不进 trait。
        for m in iface.methods.iter().filter(|m| m.is_static) {
            self.marker(m.line);
            self.emit_docs(&m.docs);
            for l in self.attr_lines(&m.attrs) {
                self.emit(&l);
            }
            let mutated = m.body.as_ref().map(|b| self.collect_mutated_with_sigs(b));
            let mut ps = Vec::new();
            for p in &m.params {
                let mut_kw = if mutated.as_ref().map_or(false, |s| s.contains(&p.name)) {
                    "mut "
                } else {
                    ""
                };
                ps.push(format!("{}{}: {}", mut_kw, p.name, self.rust_type(&p.ty)));
            }
            let ret = if m.ret == Type::Void {
                String::new()
            } else {
                format!(" -> {}", self.rust_type(&m.ret))
            };
            let sig = format!(
                "pub fn {}_{}{}({}){}",
                iface.name,
                m.name,
                impl_param_decl(&m.type_params),
                ps.join(", "),
                ret
            );
            match &m.body {
                Some(body) => {
                    self.emit(&format!("{} {{", sig));
                    self.indent += 1;
                    let prev_generic = self.generic_params.clone();
                    self.generic_params
                        .extend(m.type_params.iter().map(|p| p.name.clone()));
                    let mut env = Env::new(m.ret.clone());
                    for p in &m.params {
                        env.vars.declare(p.name.clone(), p.ty.clone());
                    }
                    env.mutated = mutated.clone().unwrap_or_default();
                    env.self_class = Some(iface.name.clone());
                    let prev = self.in_method;
                    self.in_method = false;
                    self.gen_stmts(body, &mut env);
                    self.in_method = prev;
                    self.generic_params = prev_generic;
                    self.indent -= 1;
                    self.emit("}");
                }
                None => {
                    self.cur_line = m.line;
                    self.err_code(
                        "JR3024",
                        format!("接口 static 方法 `{}` 必须有方法体", m.name),
                        "Java 的接口 static 方法不能是抽象声明；补上 `{ ... }`，或去掉 `static` 改成实例方法。",
                    );
                }
            }
            self.blank();
        }
    }

    fn gen_enum(&mut self, e: &EnumDecl) {
        self.marker(e.line);
        self.emit_docs(&e.docs);
        for l in self.attr_lines(&e.attrs) {
            self.emit(&l);
        }
        let extra: Vec<&str> = if self.cloneable.contains(&e.name) {
            vec!["Debug", "Clone"]
        } else {
            vec!["Debug"]
        };
        if let Some(d) = self.derive_line(&e.attrs, &extra) {
            self.emit(&d);
        }
        self.emit(&format!(
            "pub enum {}{} {{",
            rust_ident(&e.name),
            type_param_decl(&e.type_params)
        ));
        self.indent += 1;
        for v in &e.variants {
            self.marker(v.line);
            self.emit_docs(&v.docs);
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

        for name in &types {
            let Some(src) = std_error_source(name) else {
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
        // JR-C6：自定义异常类 → `From<X> for JrustError`，`?` 可直接传播。
        for class in &self.prog.classes {
            if class.is_exception && types.contains(&class.name) {
                self.emit(&format!(
                    "impl From<{}> for {} {{",
                    class.name, ERROR_ENUM
                ));
                self.indent += 1;
                self.emit(&format!(
                    "fn from(e: {}) -> Self {{ {}::{}(e.message) }}",
                    class.name, ERROR_ENUM, class.name
                ));
                self.indent -= 1;
                self.emit("}");
                self.blank();
            }
        }
    }

    /// 自定义异常类（JR-C6）：`class X extends Exception` → `struct X` + `Display` + `Error`。
    fn gen_exception(&mut self, class: &ClassDecl) {
        self.marker(class.line);
        self.emit_docs(&class.docs);
        for l in self.attr_lines(&class.attrs) {
            self.emit(&l);
        }
        self.cur_line = class.line;
        self.emit("#[derive(Debug, Clone)]");
        self.emit(&format!("pub struct {} {{", rust_ident(&class.name)));
        self.indent += 1;
        self.emit("pub message: String,");
        self.indent -= 1;
        self.emit("}");
        self.blank();

        self.emit(&format!("impl {} {{", rust_ident(&class.name)));
        self.indent += 1;
        self.emit("pub fn new(message: impl Into<String>) -> Self {");
        self.indent += 1;
        self.emit("Self { message: message.into() }");
        self.indent -= 1;
        self.emit("}");
        self.indent -= 1;
        self.emit("}");
        self.blank();

        self.emit(&format!("impl std::fmt::Display for {} {{", class.name));
        self.indent += 1;
        self.emit("fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {");
        self.indent += 1;
        self.emit(&format!(
            "write!(f, \"{}: {{}}\", self.message)",
            class.name
        ));
        self.indent -= 1;
        self.emit("}");
        self.indent -= 1;
        self.emit("}");
        self.emit(&format!("impl std::error::Error for {} {{}}", class.name));
        self.blank();
    }

    /// `static final` 字段 → 模块级 `pub const`（JR-D8）。
    fn gen_consts(&mut self, class: &ClassDecl) {
        let consts: Vec<&Field> = class.fields.iter().filter(|f| is_const_field(f)).collect();
        for f in &consts {
            self.marker(f.line);
            self.emit_docs(&f.docs);
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

    /// 注解 → `#[...]` 行（JR-D9）。`derive` 单独合并，`raw` 原样输出。
    /// `///` 文档注释（JR-L2）：原样输出。
    fn emit_docs(&mut self, docs: &[String]) {
        for d in docs {
            self.emit(&format!("///{}", d));
        }
    }

    fn attr_lines(&self, attrs: &[Attr]) -> Vec<String> {
        let mut out = Vec::new();
        for a in attrs {
            match a.name.as_str() {
                "derive" => {}
                // `@shared` 由字段类型消费（JR-C13），不是 Rust 属性，不能透传。
                "shared" => {}
                "raw" => {
                    if let Some(s) = &a.args {
                        if let Some(inner) = strip_str_literal(s) {
                            out.extend(inner.lines().map(|l| l.trim().to_string()));
                        }
                    }
                }
                _ => match &a.args {
                    Some(x) => out.push(format!("#[{}({})]", a.name, x)),
                    None => out.push(format!("#[{}]", a.name)),
                },
            }
        }
        out
    }

    /// 合并 `@derive(...)` 与自动推导（Clone / Debug），输出单条 `#[derive(...)]`。
    fn derive_line(&self, attrs: &[Attr], extra: &[&str]) -> Option<String> {
        let mut names: Vec<String> = Vec::new();
        for a in attrs.iter().filter(|a| a.name == "derive") {
            if let Some(args) = &a.args {
                for part in args.split(',') {
                    let p = part.trim();
                    if !p.is_empty() && !names.iter().any(|n| n == p) {
                        names.push(p.to_string());
                    }
                }
            }
        }
        for e in extra {
            if !names.iter().any(|n| n == e) {
                names.push((*e).to_string());
            }
        }
        if names.is_empty() {
            None
        } else {
            Some(format!("#[derive({})]", names.join(", ")))
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
        self.emit_docs(&class.docs);
        for l in self.attr_lines(&class.attrs) {
            self.emit(&l);
        }
        let extra: Vec<&str> = if self.cloneable.contains(&class.name) {
            vec!["Clone"]
        } else {
            Vec::new()
        };
        if let Some(d) = self.derive_line(&class.attrs, &extra) {
            self.emit(&d);
        }
        if instance_fields(class).next().is_none() && phantom.is_empty() {
            self.emit(&format!("pub struct {}{};", rust_ident(&class.name), tparams));
        } else {
            self.emit(&format!("pub struct {}{} {{", rust_ident(&class.name), tparams));
            self.indent += 1;
            for f in instance_fields(class) {
                self.marker(f.line);
                self.emit_docs(&f.docs);
                for l in self.attr_lines(&f.attrs) {
                    self.emit(&l);
                }
                let vis = if f.vis == Vis::Public { "pub " } else { "" };
                let ty = if field_is_shared(f) {
                    self.shared_rust_type(&class.name, f)
                } else {
                    self.rust_type(&f.ty)
                };
                self.emit(&format!("{}{}: {},", vis, f.name, ty));
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
        // 接口可能声明在别的文件里，查表要用全工程集合。
        let prog = self.types;
        // 展开接口继承（JR-D6）：实现子接口时，祖先接口也要生成 impl。
        let mut impl_ifaces: Vec<Type> = Vec::new();
        let mut seen_ifaces: HashSet<String> = HashSet::new();
        for ity in &class.implements {
            collect_ifaces(ity.clone(), prog, &mut impl_ifaces, &mut seen_ifaces);
        }
        let mut iface_methods: HashSet<String> = HashSet::new();
        for ity in &impl_ifaces {
            if let Some(iface) = prog.interfaces.iter().find(|x| x.name == type_name(ity)) {
                for m in &iface.methods {
                    iface_methods.insert(m.name.clone());
                }
            }
        }

        // 实现接口的方法 → impl Trait for Type
        for ity in &impl_ifaces {
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
        // `public static async void main(...)` → `#[tokio::main] async fn main()`（JR-S5）。
        if main.is_async {
            self.emit("#[tokio::main]");
        }
        if throws {
            let a = if main.is_async { "async " } else { "" };
            self.emit(&format!("{}fn main() -> Result<(), {}> {{", a, ERROR_ENUM));
        } else if main.is_async {
            self.emit("async fn main() {");
        } else {
            self.emit("fn main() {");
        }
        self.indent += 1;
        if stmts_use_ident(&main.body, "args") {
            self.emit("let args: Vec<String> = std::env::args().skip(1).collect();");
        }
        let mut env = Env::new(Type::Void);
        for p in &main.params {
            env.vars.declare(p.name.clone(), p.ty.clone());
        }
        env.mutated = self.collect_mutated_with_sigs(&main.body);
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
        self.emit_docs(&ctor.docs);
        for l in self.attr_lines(&ctor.attrs) {
            self.emit(&l);
        }
        self.emit(&format!("{}fn new({}) -> Self {{", vis, ps.join(", ")));
        self.indent += 1;
        let mut env = Env::new(Type::Void);
        for p in &ctor.params {
            env.vars.declare(p.name.clone(), p.ty.clone());
        }
        let mut fields: Vec<String> = Vec::new();
        for st in &ctor.body {
            match st {
                Stmt::Assign {
                    target: Expr::Field(recv, field),
                    value,
                    ..
                } if matches!(**recv, Expr::This(_)) => {
                    if class.fields.iter().any(|f| &f.name == field && is_const_field(f)) {
                        self.cur_line = ctor.line;
                        self.err(
                            format!("不能给静态常量 `{}` 赋值", field),
                            "static final 转发为 `const`，编译期固定；构造器里只给实例字段赋值。",
                        );
                        continue;
                    }
                    let fdecl = instance_fields(class).find(|f| &f.name == field).cloned();
                    let field_ty = fdecl.as_ref().map(|f| f.ty.clone());
                    self.cur_line = ctor.line;
                    let mut v = match &field_ty {
                        Some(ft) => self.gen_assigned_value(value, ft, &mut env, false, None),
                        None => self.gen_expr(value, &mut env, None),
                    };
                    // `@shared` 字段：构造时包成 `Arc::new(..)` / `Arc::new(Mutex::new(..))`（JR-C13）。
                    if let Some(fd) = &fdecl {
                        if field_is_shared(fd) {
                            v = if self.shared_is_mut(&class.name, field) {
                                format!("std::sync::Arc::new(std::sync::Mutex::new({}))", v)
                            } else {
                                format!("std::sync::Arc::new({})", v)
                            };
                        }
                    }
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
        let mutated = self.collect_mutated_with_sigs(&m.body);
        let recv_mut = self.sigs.recv(class_name, &m.name) == Recv::Mut;
        let modes = self.param_modes(class_name, &m.name);
        let rs = self.ret_sig(class_name, &m.name, &m.params, &modes, recv_mut);
        let mut ps = Vec::new();
        if !m.is_static {
            ps.push(match (recv_mut, rs.self_ann) {
                (true, _) => "&mut self".to_string(),
                (false, true) => "&'a self".to_string(),
                (false, false) => "&self".to_string(),
            });
        }
        for (i, p) in m.params.iter().enumerate() {
            let raw = modes.get(i).copied().unwrap_or(PMode::Value);
            let mode = self.effective_mode(raw, &p.ty);
            let mut_kw = if mutated.contains(&p.name) { "mut " } else { "" };
            let lt = if rs.param_ann.contains(&i) {
                rs.lt.as_deref()
            } else {
                None
            };
            ps.push(format!(
                "{}{}: {}",
                mut_kw,
                p.name,
                self.param_type_lt(mode, &p.ty, lt)
            ));
        }
        let ret_borrow = rs.borrow;
        let ret = if ret_borrow {
            rs.ret_type()
        } else if m.ret == Type::Void {
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
        self.emit_docs(&m.docs);
        for l in self.attr_lines(&m.attrs) {
            self.emit(&l);
        }
        self.emit(&format!(
            "{}{}fn {}{}({}){} {{",
            vis,
            if m.is_async { "async " } else { "" },
            m.name,
            rs.generic_decl(&m.type_params),
            ps.join(", "),
            ret
        ));
        self.indent += 1;
        let prev_generic = self.generic_params.clone();
        self.generic_params.extend(m.type_params.iter().map(|p| p.name.clone()));
        let mut env = Env::new(m.ret.clone());
        for (i, p) in m.params.iter().enumerate() {
            env.vars.declare(p.name.clone(), p.ty.clone());
            let raw = modes.get(i).copied().unwrap_or(PMode::Value);
            let mode = self.effective_mode(raw, &p.ty);
            if !mode.is_value() {
                env.borrowed.insert(p.name.clone(), mode);
            }
        }
        env.ret_borrow = ret_borrow;
        env.mutated = mutated;
        env.in_throws = throws;
        env.self_class = Some(class_name.to_string());
        let prev = self.in_method;
        let prev_async = self.in_async;
        self.in_method = !m.is_static;
        self.in_async = m.is_async;
        // `synchronized` 方法（JR-C12）：方法体开头按字段名排序依次锁定类内
        // 所有 `Arc<Mutex<T>>` 的 `@shared` 字段，避免多个方法加锁顺序不一致导致死锁。
        let locked = if m.is_synchronized {
            self.lock_shared_fields(class_name)
        } else {
            Vec::new()
        };
        if m.is_synchronized && locked.is_empty() {
            self.emit("// jrust 提示: synchronized 方法内没有可锁的 @shared 可变字段，按普通方法生成。");
        }
        self.gen_stmts(&m.body, &mut env);
        for f in &locked {
            self.guarded.remove(f);
        }
        self.in_async = prev_async;
        self.in_method = prev;
        self.generic_params = prev_generic;
        if throws && m.ret == Type::Void {
            self.emit("Ok(())");
        }
        self.indent -= 1;
        self.emit("}");
    }

    // ---------- 语句 ----------

    /// 需要 `let mut` / `mut 形参` 的变量：赋值目标 + 以其为接收者调用 `&mut self` 方法
    /// 或集合可变方法（如 `list.add(..)`）。名字级匹配，宁可多写 `mut`（只影响警告）。
    fn collect_mutated_with_sigs(&self, stmts: &[Stmt]) -> HashSet<String> {
        let mut set = collect_mutated(stmts);
        if self.sigs.is_empty() {
            return set;
        }
        let mut_names = self.sigs.mut_names();
        each_stmt_expr(stmts, &mut |e| {
            if let Expr::Call {
                recv: Some(r),
                name,
                ..
            } = e
            {
                if mut_names.contains(name) {
                    if let Expr::Ident(n) = &**r {
                        set.insert(n.clone());
                    }
                }
            }
        });
        // `&mut x` 实参：名字级匹配（多写 `mut` 只产生 warning）。
        let positions = self.sigs.mutref_positions();
        each_stmt_expr(stmts, &mut |e| {
            if let Expr::Call { name, args, .. } = e {
                if let Some(ps) = positions.get(name) {
                    for i in ps {
                        if let Some(Expr::Ident(n)) = args.get(*i).map(strip_parens_expr) {
                            set.insert(n.clone());
                        }
                    }
                }
            }
        });
        set
    }

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
                    mut_kw,
                    rust_ident(name),
                    ty_str,
                    init_str
                ));
                env.vars.declare(name.clone(), ty.clone());
            }
            Stmt::Assign {
                target,
                value,
                line,
            } => {
                self.cur_line = *line;
                self.marker(*line);
                // 写 `@shared` 字段（JR-C13）：先拿一次锁，避免 `+=` 的读与写在同一个表达式里
                // 二次加锁（std::sync::Mutex 不可重入，会死锁）。
                let shared_root = env.self_class.clone().and_then(|cn| {
                    target_writes_this_field(target).and_then(|f| {
                        if self.is_shared_field(&cn, &f) && !self.guarded.contains_key(&f) {
                            if self.shared_is_mut(&cn, &f) {
                                Some(f)
                            } else {
                                self.err_code(
                                    "JR3025",
                                    format!("只读 `@shared` 字段 `{}` 不能赋值", f),
                                    "这类字段在产物里是 `Arc<T>`；类里出现写操作后会自动升级为 `Arc<Mutex<T>>`，或去掉 `@shared`。",
                                );
                                None
                            }
                        } else {
                            None
                        }
                    })
                });
                if let Some(fld) = shared_root {
                    self.emit("{");
                    self.indent += 1;
                    self.emit(&format!("let mut __g = self.{}.lock().unwrap();", fld));
                    self.guarded.insert(fld.clone(), "__g".to_string());
                    let target_ty = self.target_type(target, env);
                    let t = self.gen_expr(target, env, None);
                    let v = match &target_ty {
                        Some(tt) => self.gen_assigned_value(value, tt, env, true, Some(&t)),
                        None => self.gen_expr(value, env, None),
                    };
                    self.guarded.remove(&fld);
                    self.emit(&format!("{} = {};", t, v));
                    self.indent -= 1;
                    self.emit("}");
                    return;
                }
                let target_ty = self.target_type(target, env);
                if let Expr::Ident(n) = strip_parens_expr(target) {
                    // 重新赋值等于重新获得所有权，之后可以继续使用。
                    env.moved.remove(n);
                }
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
                        let v = if env.ret_borrow {
                            // `-> &T`：来源本身就是引用（形参 / 调用结果），字段访问才补 `&`。
                            let raw = self.gen_expr(e, env, None);
                            if matches!(strip_parens_expr(e), Expr::Field(..)) {
                                format!("&{}", raw)
                            } else {
                                raw
                            }
                        } else {
                            // 返回 `this` 的字段时从 `&self` 搬不走，必须复制。
                            let copy = self.is_self_field(e, env);
                            self.gen_assigned_value(e, &ret, env, copy, None)
                        };
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
                env.vars.push_scope();
                self.gen_stmts(then, env);
                env.vars.pop_scope();
                self.indent -= 1;
                if let Some(else_body) = els {
                    self.emit("} else {");
                    self.indent += 1;
                    env.vars.push_scope();
                    self.gen_stmts(else_body, env);
                    env.vars.pop_scope();
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
                env.vars.push_scope();
                self.gen_stmts(body, env);
                env.vars.pop_scope();
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
                // `for (T x : c.mutable())` → `for x in c.iter_mut()`（JR-E4）。
                let mut_base = match strip_parens_expr(iter) {
                    Expr::Call {
                        recv: Some(base),
                        name: n,
                        args,
                    } if n == "mutable" && args.is_empty() => Some(&**base),
                    _ => None,
                };
                let it = if let Some(base) = mut_base {
                    let ok = matches!(
                        self.infer_type(base, env),
                        Some(Type::List(_)) | Some(Type::Array(_))
                    );
                    if ok {
                        format!("{}.iter_mut()", self.gen_expr(base, env, None))
                    } else {
                        self.err_code(
                            "JR3028",
                            "`mutable()` 只适用于 List<T> / T[]，用于需要修改元素时",
                            "Set 的元素是哈希键，不能就地改；先 `remove` 再 `add`。只读遍历直接写 `for (T x : c)`。",
                        );
                        format!("&{}", self.gen_expr(base, env, None))
                    }
                } else if self.is_borrowed_value(iter, env)
                    && matches!(
                        self.infer_type(iter, env),
                        Some(Type::List(_)) | Some(Type::Array(_)) | Some(Type::Set(_))
                    )
                {
                    // 借用形参本身已是引用：`&p` 会变成 `&&[T]`，直接 `p.iter()`。
                    format!("{}.iter()", self.gen_expr(iter, env, None))
                } else {
                    format!("&{}", self.gen_expr(iter, env, None))
                };
                self.emit(&format!("for {} in {} {{", name, it));
                self.indent += 1;
                env.vars.push_scope();
                env.vars.declare(name.clone(), ty.clone());
                self.gen_stmts(body, env);
                env.vars.pop_scope();
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
                env.vars.push_scope();
                env.vars.declare(name.clone(), ty.clone());
                self.gen_stmts(body, env);
                env.vars.pop_scope();
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
            Stmt::Spawn { body, line } => {
                self.cur_line = *line;
                self.marker(*line);
                // Stage E：`async move` 会把捕获的变量整个搬进闭包，外层再用就 `E0382`。
                // 做法：spawn 之前为每个自由变量生成 `let __jr_sp<N>_x = x.clone();`，
                // 闭包内再 `let x = __jr_sp<N>_x;` 把名字接回来。这样 `async move`
                // 捕获的是副本，外层原值仍在（与 Java lambda 捕获局部变量是副本一致）。
                // 注意不能用 `let x = x.clone();` 的同名 shadowing —— 那只是给同一块内存
                // 起了第二个名字，`async move` 照样把外层 `x` 移走。
                let outer: Vec<String> = env.vars.visible_names();
                let captures = spawn_free_vars(body, &outer);
                let mut rebind: Vec<(String, String)> = Vec::new();
                for name in captures.iter() {
                    let ty = env.vars.lookup(name).cloned();
                    // Copy 标量由 `async move` 直接复制，不需要（也不该）clone。
                    if ty.as_ref().map_or(false, |t| t.is_copy_scalar()) {
                        continue;
                    }
                    // 借用形参不会走到这里：`spawn` 捕获本身是 Value 用法（参数三态会推成
                    // owned），显式 `@ref` / `@mut` 又被 Stage D 的 `JR4001` 先拦下。
                    if !ty.as_ref().map_or(true, |t| self.is_cloneable_type(t)) {
                        self.err_code(
                            "JR4002",
                            format!("`spawn` 块捕获的 `{}` 不可 Clone，无法复制进闭包", name),
                            "它含有 trait 对象等不可克隆字段；改成可 Clone 的数据结构，或把需要的部分拆成可复制的局部变量。",
                        );
                        continue;
                    }
                    let tmp = format!("__jr_sp{}_{}", self.spawn_seq, name);
                    self.emit(&format!("let {} = {}.clone();", tmp, name));
                    rebind.push((name.clone(), tmp));
                }
                self.spawn_seq += 1;
                // `spawn` 的闭包按值捕获：`self` 是 `&self`，不满足 'static（JR3023）。
                // 记录块内**首个** `this` 出现的行，诊断定位到那里而不是块尾。
                let prev_this = self.this_line.take();
                let prev_async = self.in_async;
                self.in_async = true;
                self.emit("tokio::spawn(async move {");
                self.indent += 1;
                for (name, tmp) in &rebind {
                    self.emit(&format!("let {} = {};", name, tmp));
                }
                env.vars.push_scope();
                self.gen_stmts(body, env);
                env.vars.pop_scope();
                self.indent -= 1;
                self.emit("});");
                self.in_async = prev_async;
                let captured_this = self.this_line.take();
                self.this_line = prev_this;
                if let Some(line) = captured_this {
                    let saved = self.cur_line;
                    self.cur_line = line;
                    self.err_code(
                        "JR3023",
                        "`spawn` 块捕获了 `this`，它不满足 Send + 'static",
                        "`this` 是 `&self`，不能跨线程持有；先把需要的字段复制到局部变量（标量 / String / Vec 可 clone）再 spawn。",
                    );
                    self.cur_line = saved;
                }
            }
            Stmt::Sync { lock, body, line } => {
                self.cur_line = *line;
                self.marker(*line);
                let field = match strip_parens_expr(lock) {
                    Expr::Field(recv, f) if matches!(**recv, Expr::This(_)) => Some(f.clone()),
                    _ => None,
                };
                let valid = field.as_ref().map_or(false, |f| {
                    env.self_class
                        .as_deref()
                        .map_or(false, |cn| self.is_shared_field(cn, f) && self.shared_is_mut(cn, f))
                });
                if let (Some(f), true) = (field, valid) {
                    // 已持锁（synchronized 方法 / 外层 synchronized 块）时不再重复加锁。
                    if self.guarded.contains_key(&f) {
                        self.gen_stmts(body, env);
                    } else {
                        self.emit("{");
                        self.indent += 1;
                        self.emit(&format!("let mut _g = self.{}.lock().unwrap();", f));
                        self.guarded.insert(f.clone(), "_g".to_string());
                        env.vars.push_scope();
                        self.gen_stmts(body, env);
                        env.vars.pop_scope();
                        self.guarded.remove(&f);
                        self.indent -= 1;
                        self.emit("}");
                    }
                } else {
                    self.err_code(
                        "JR3026",
                        "`synchronized` 需要一个 `@shared` 可变字段作为锁",
                        "写成 `synchronized (this.f) { ... }`；`f` 要声明为 `@shared`，且类内方法写过它（才会是 `Arc<Mutex<T>>`）。",
                    );
                    self.gen_stmts(body, env);
                }
            }
            Stmt::Raw { code, line } => {
                self.cur_line = *line;
                self.emit_raw(code, *line);
            }
            Stmt::Throw { expr, line } => {
                self.cur_line = *line;
                self.marker(*line);
                // JR-C6：`IllegalArgumentException` 属于编程错误，Rust 里用 panic! 表达，
                // 不需要 `throws`，也不进入错误枚举。
                if let Expr::New { class, args, .. } = expr {
                    if simple_type_name(class) == "IllegalArgumentException" {
                        let msg = match args.len() {
                            0 => "\"\"".to_string(),
                            1 => self.gen_expr(&args[0], env, Some(&Type::String)),
                            _ => {
                                self.err_code(
                                    "JR3021",
                                    "`throw new IllegalArgumentException` 只支持 0 或 1 个参数",
                                    "把补充信息拼进同一个字符串。",
                                );
                                "\"\"".to_string()
                            }
                        };
                        self.emit("// jrust 提示: IllegalArgumentException 是编程错误，Rust 里用 panic! 表达");
                        self.emit(&format!("panic!(\"{{}}\", {});", msg));
                        return;
                    }
                }
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
                self.gen_switch(subject, cases, default, *line, env);
            }
        }
    }

    /// `switch` → Rust `match`，并做穷尽性 / fallthrough 诊断。
    fn gen_switch(
        &mut self,
        subject: &Expr,
        cases: &[SwitchCase],
        default: &Option<Vec<Stmt>>,
        line: usize,
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
            // 报在 `switch` 行本身，而不是最后一个 case 体的末行。
            self.cur_line = line;
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
            Expr::Ident(n) => {
                if env.moved.contains(n) {
                    self.err_code(
                        "JR4001",
                        format!("变量 `{}` 的所有权已经移走，不能再使用", n),
                        "它作为 owned 实参传出去后调用方不再持有；要保留一份就先写 `clone()`，或把被调方法的形参改成只读 / 借用用法。",
                    );
                }
                rust_ident(n)
            }
            Expr::This(line) => {
                // 记录首个 `this` 的源码行（而非语句起始行），供 JR3023 精确定位。
                if self.this_line.is_none() {
                    self.this_line = Some(*line);
                }
                "self".to_string()
            }
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
                            return format!("{}::{}", rust_ident(en), rust_ident(name));
                        }
                    }
                }
                // `@shared` 字段（JR-C13）：读走 Arc / Mutex 解引用；块内已持锁时直接用 guard。
                if matches!(**recv, Expr::This(_)) {
                    if let Some(cn) = env.self_class.clone() {
                        if self.is_shared_field(&cn, name) {
                            if let Some(g) = self.guarded.get(name).cloned() {
                                return format!("(*{})", g);
                            }
                            return if self.shared_is_mut(&cn, name) {
                                format!("(*self.{}.lock().unwrap())", name)
                            } else {
                                format!("(*self.{})", name)
                            };
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
                // 二元运算的标量提升（阶段 C / JR-T9 延伸）：Java 里 `int + double` 结果是
                // `double`，Rust 里 `i32 + f64` 直接 E0277。按 Java 规则把较窄的一侧加宽。
                // 只处理算术 / 关系运算；`&&` / `||` 两侧必须是 bool，不参与提升。
                if matches!(
                    op,
                    BinOp::Add
                        | BinOp::Sub
                        | BinOp::Mul
                        | BinOp::Div
                        | BinOp::Rem
                        | BinOp::Lt
                        | BinOp::Le
                        | BinOp::Gt
                        | BinOp::Ge
                ) {
                    if let Some((l2, r2)) = self.promote_binary(&l, &r, lhs, rhs, env) {
                        return format!("{} {} {}", l2, binop_str(*op), r2);
                    }
                }
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
            Expr::Await(inner) => {
                if !self.in_async {
                    self.err_code(
                        "JR3022",
                        "`await` 只能出现在 async 方法里",
                        "给当前方法加 `async` 修饰符，或去掉 `await`（同步调用会直接返回结果）。",
                    );
                }
                let x = self.gen_expr(inner, env, None);
                format!("{}.await", x)
            }
            Expr::Raw { code, line } => {
                // 表达式位置：保留源映射标记（JR-X1）。
                //
                // 单行：`(/* jrust:L */ x)` —— 保持单行，便于与源码对拍。
                // （产物不做 rustfmt 归一化，不承诺整份文件 `rustfmt --check` 通过，见 SPEC §15。）
                // 多行：必须保留块表达式外壳 `{ … }`。多行 raw 体内常有 `let` 等语句，
                // `(/* jrust:L */let x = 1; x)` 不是合法表达式（rustc: expected expression）。
                let body = code.trim_end();
                if body.contains('\n') {
                    let pad = " ".repeat(self.indent * 4);
                    format!("(/* jrust:L{} */{{{}\n{}}})", line, body, pad)
                } else {
                    format!("(/* jrust:L{} */{})", line, body.trim())
                }
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
                    .lookup(n)
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
                if matches!(**recv, Expr::This(_)) {
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
            // 已经是 `&str` 的（借用形参 / getter 返回）也直接传。
            Some(e) if self.is_borrowed_value(e, env) || self.call_returns_str_ref(e, env) => {
                self.gen_expr(e, env, None)
            }
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
        if matches!(class, "HashSet") {
            if !args.is_empty() {
                self.err(
                    "jrust MVP 只支持无参的 new HashSet<>()",
                    "Rust 用 `HashSet::new()` 再逐个 `insert`。",
                );
            }
            return "HashSet::new()".to_string();
        }
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
                Some(pt) => arg_strs.push(self.gen_owned_arg(a, pt, env)),
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
        let cname = rust_ident(class);
        let Some((_, _, has_ctor, needs_phantom)) = info else {
            return format!("{}{}::new({})", cname, turbofish, arg_strs.join(", "));
        };
        if has_ctor {
            format!("{}{}::new({})", cname, turbofish, arg_strs.join(", "))
        } else if args.is_empty() {
            if needs_phantom {
                format!("{} {{ _marker: std::marker::PhantomData }}", cname)
            } else {
                cname
            }
        } else {
            self.err(
                format!("类 {} 没有构造器却传入了参数", class),
                "为它写一个构造器（方法名与类名相同）；Rust 里对应 `impl X { pub fn new(...) -> Self }`。",
            );
            cname
        }
    }

    /// 迭代器链白名单（JR-E18）。
    fn is_iter_step(name: &str) -> bool {
        matches!(name, "map" | "filter" | "collectList")
    }

    /// `e` 是否是 `x.stream()....` 链（基座或中间步骤）。
    fn is_iter_chain(e: &Expr) -> bool {
        match e {
            Expr::Call {
                recv: Some(inner),
                name,
                ..
            } => name == "stream" || Self::is_iter_step(name) || Self::is_iter_chain(inner),
            _ => false,
        }
    }

    /// 沿调用链一路走到基座表达式（`xs.stream().map(..)` → `xs`）。
    fn chain_base(mut e: &Expr) -> &Expr {
        while let Expr::Call {
            recv: Some(inner), ..
        } = e
        {
            e = inner;
        }
        e
    }

    /// 生成迭代器链的一步（JR-E18）：`map` / `filter` 透传，`collectList` 收集成 `Vec`。
    fn gen_iter_step(
        &mut self,
        recv: &Expr,
        name: &str,
        args: &[Expr],
        env: &mut Env,
    ) -> String {
        let base = self.gen_expr(recv, env, None);
        match name {
            "map" | "filter" => {
                if args.len() != 1 {
                    self.err(
                        format!("`{}` 需要一个闭包参数", name),
                        format!("例如 `xs.stream().{}(x -> x * 2).collectList()`。", name),
                    );
                    return format!("{}.{}()", base, name);
                }
                let f = self.gen_expr(&args[0], env, None);
                format!("{}.{}({})", base, name, f)
            }
            "collectList" => format!("{}.collect::<Vec<_>>()", base),
            other => {
                self.err_code(
                    "JR3017",
                    format!("迭代器链不支持方法 `{}`", other),
                    "MVP 白名单：stream / map / filter / collectList。",
                );
                format!("{}.{}()", base, other)
            }
        }
    }

    /// `Cls.method` / `Iface.method` 对应静态方法时返回（Rust 调用路径，形参类型）（JR-D10）。
    fn static_method(&self, ty: &str, name: &str) -> Option<(String, Vec<Type>)> {
        if let Some(cd) = self.classes.get(ty) {
            if let Some(m) = cd.methods.iter().find(|m| m.is_static && m.name == name) {
                return Some((
                    format!("{}::{}", rust_ident(ty), rust_ident(name)),
                    m.params.iter().map(|p| p.ty.clone()).collect(),
                ));
            }
        }
        self.types
            .interfaces
            .iter()
            .find(|i| i.name == ty)
            .and_then(|i| i.methods.iter().find(|m| m.is_static && m.name == name))
            .map(|m| {
                (
                    format!("{}_{}", ty, name),
                    m.params.iter().map(|p| p.ty.clone()).collect(),
                )
            })
    }

    fn gen_call(
        &mut self,
        recv: &Option<Box<Expr>>,
        name: &str,
        args: &[Expr],
        env: &mut Env,
        expect: Option<&Type>,
    ) -> String {
        // 迭代器链（JR-E18）：链上的每一步都交给 gen_iter_step。
        if let Some(r) = recv.as_deref() {
            if Self::is_iter_chain(r) {
                return self.gen_iter_step(r, name, args, env);
            }
        }
        if (name == "println" || name == "print") && recv.as_ref().map_or(true, |r| is_system_out(r)) {
            let mac = if name == "println" { "println" } else { "print" };
            return self.gen_print_macro(args, env, mac);
        }

        // 内置 `panic("msg")` / `assert(cond, "msg")` → `panic!` / `assert!`（JR-C7）。
        if recv.is_none() && (name == "panic" || name == "assert") {
            if name == "panic" {
                if args.len() != 1 {
                    self.err(
                        "`panic` 需要一个字符串消息",
                        "写成 `panic(\"原因\")`，产物是 `panic!(\"{}\", 原因)`。",
                    );
                }
                let msg = args
                    .first()
                    .map(|a| self.gen_expr(a, env, None))
                    .unwrap_or_else(|| "\"\"".to_string());
                return format!("panic!(\"{{}}\", {})", msg);
            }
            if args.is_empty() || args.len() > 2 {
                self.err(
                    "`assert` 需要 1~2 个参数",
                    "写成 `assert(条件, \"失败原因\")`；产物是 `assert!(条件, \"{}\", 原因)`。",
                );
            }
            let cond = match args.first() {
                Some(a) => self.gen_expr(a, env, Some(&Type::Boolean)),
                None => "false".to_string(),
            };
            return match args.get(1) {
                Some(m) => {
                    let msg = self.gen_expr(m, env, None);
                    format!("assert!({}, \"{{}}\", {})", cond, msg)
                }
                None => format!("assert!({})", cond),
            };
        }

        if recv.is_none() {
            self.err(
                format!("未知的函数 `{}`", name),
                "jrust 没有自由函数：裸调用只支持 `println` / `print` / `panic` / `assert`；其它逻辑写成对象或类的方法。",
            );
            return format!("{}()", name);
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
                        Some(pt) => vals.push(self.gen_owned_arg(a, pt, env)),
                        None => vals.push(self.gen_expr(a, env, None)),
                    }
                }
                return if vals.is_empty() {
                    format!("{}::{}", rust_ident(en), rust_ident(name))
                } else {
                    format!("{}::{}({})", rust_ident(en), rust_ident(name), vals.join(", "))
                };
            }
        }

        // `Cls.method(...)` / `Iface.method(...)`：静态方法 → 关联函数（JR-D10）。
        if let Some(Expr::Ident(cn)) = recv.as_deref() {
            if let Some((path, params)) = self.static_method(cn, name) {
                let mut vals = Vec::new();
                for (i, a) in args.iter().enumerate() {
                    match params.get(i) {
                        Some(pt) => vals.push(self.gen_owned_arg(a, pt, env)),
                        None => vals.push(self.gen_expr(a, env, None)),
                    }
                }
                return format!("{}({})", path, vals.join(", "));
            }
        }

        // 多文件模式（JR-S4/M3）：别的模块里的类 / 枚举在这里看不到签名，
        // 按 Java 命名习惯把大写开头的接收者当成类型名，生成关联函数调用。
        if self.multi_file {
            if let Some(Expr::Ident(cn)) = recv.as_deref() {
                let type_like = cn.chars().next().map_or(false, |c| c.is_ascii_uppercase())
                    && !env.vars.contains(cn);
                if type_like {
                    let vals: Vec<String> = args
                        .iter()
                        .map(|a| match a {
                            // 跨模块看不到形参类型；jrust 里的字符串参数只会是 String。
                            Expr::Str(_) => self.gen_expr(a, env, Some(&Type::String)),
                            _ => self.gen_expr(a, env, None),
                        })
                        .collect();
                    if vals.is_empty() {
                        return format!("{}::{}()", cn, name);
                    }
                    return format!("{}::{}({})", cn, name, vals.join(", "));
                }
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
        if let Some(Type::Set(elem_ty)) = &recv_ty {
            let r = self.gen_expr(recv.as_ref().unwrap(), env, None);
            match name {
                "size" => return format!("{}.len()", r),
                "isEmpty" => return format!("{}.is_empty()", r),
                "add" => {
                    if args.len() != 1 {
                        self.err("add 需要一个元素", "例如 `s.add(1)`；Rust 对应 `insert`。");
                    }
                    let v = args
                        .first()
                        .map(|a| self.gen_owned_arg(a, elem_ty, env))
                        .unwrap_or_default();
                    return format!("{}.insert({})", r, v);
                }
                // contains / remove 用引用查表：String 元素走 `as_str()`，其余加 `&`。
                "contains" => {
                    if args.len() != 1 {
                        self.err("contains 需要一个元素", "例如 `s.contains(1)`。");
                    }
                    let v = self.gen_map_key(args.first(), elem_ty, env);
                    return format!("{}.contains({})", r, v);
                }
                "remove" => {
                    if args.len() != 1 {
                        self.err("remove 需要一个元素", "例如 `s.remove(1)`。");
                    }
                    let v = self.gen_map_key(args.first(), elem_ty, env);
                    return format!("{}.remove({})", r, v);
                }
                other => {
                    self.err(
                        format!("Set 没有方法 `{}`", other),
                        "MVP 支持 add / contains / remove / size / isEmpty。",
                    );
                    return format!("{}.{}()", r, other);
                }
            }
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
                        .map(|a| self.gen_owned_arg(a, k_ty, env))
                        .unwrap_or_default();
                    let v = args
                        .get(1)
                        .map(|a| self.gen_owned_arg(a, v_ty, env))
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
                    // JR9001（双轨期第一步，SPEC-v2 §3-1）：`map.get(k)` 在 2.0 将改为返回
                    // `Option<V>`（键缺失为 None，不再 panic）。当前语义仍是 `.unwrap()`，
                    // **不提前切换**（JR-V7 禁止「文档写 None、实现仍 panic」的反向不一致，
                    // 同样也禁止实现先跑）。
                    //
                    // 这里只发弃用警告，产物形态完全不变——双轨期的意义是给用户迁移时间，
                    // 不是提前改变行为。2.0 才切换 `get` 语义并删除本分支。
                    self.warn_code(
                        "JR9001",
                        "`map.get(k)` 将在 2.0 改为返回 `Option<V>`（键缺失为 `None`，不再 panic）",
                        "迁移到 `getOrNull(k)`：它现在就是 `Option<V>` 语义（产物 `get(&k).cloned()`）。写成 `V? v = m.getOrNull(\"k\");`，2.0 之后两种写法等价。",
                    );
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
        let is_list = matches!(recv_ty, Some(Type::List(_)) | Some(Type::Array(_)));        let elem_is_trait = match &recv_ty {
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
        let sig = self.call_sig(recv, name, env);

        // JR4101（D8）：接收者类型已知但本工程里没有它的源码（外部 crate）→ 签名无法判定。
        // 实参按只读借用生成，并提示用 `@ref` / `@mut` 显式覆盖。
        let unknown_external = sig.is_none() && self.is_external_recv_type(recv_ty.as_ref());
        if unknown_external {
            let tn = recv_ty
                .as_ref()
                .map(|t| self.rust_type(t))
                .unwrap_or_else(|| "?".to_string());
            self.warn_code(
                "JR4101",
                format!(
                    "无法确定 `{}.{}` 的形参借用方式，实参按只读借用（`&T`）生成",
                    tn, name
                ),
                "外部 crate 的类型没有源码签名：确认这些实参在 Rust 侧确实只需要 `&T`；需要可变借用时给对应形参标 `@mut`。",
            );
        }

        if is_list && name == "stream" {
            let r = self.gen_expr(recv.as_ref().unwrap(), env, None);
            // 值语义：`iter()` 产出 `&T`，`cloned()` 还原成 `T`；否则 `filter` 里是 `&&T`、
            // `collect` 得到 `Vec<&T>`，与 Java 的按值流不一致（JR-E18）。
            return format!("{}.iter().cloned()", r);
        }

        let mut arg_strs = Vec::new();
        for (i, a) in args.iter().enumerate() {
            if is_list && name == "add" {
                if let Some(et) = &elem_ty {
                    if matches!(et, Type::Nullable(_)) {
                        arg_strs.push(self.gen_owned_arg(a, et, env));
                        continue;
                    }
                }
                let inner = self.gen_expr(a, env, None);
                if elem_is_trait {
                    arg_strs.push(format!("Box::new({})", inner));
                } else if elem_is_string {
                    // Vec<String> 的 push 需要 String，字面量要补 to_string()
                    arg_strs.push(format!("({}).to_string()", inner));
                } else if let Some(et) = &elem_ty {
                    arg_strs.push(self.gen_owned_arg(a, et, env));
                } else {
                    arg_strs.push(inner);
                }
                continue;
            }
            let pt = param_types.as_ref().and_then(|p| p.get(i));
            let mode = sig.as_ref().and_then(|s| s.0.get(i)).copied();
            match (pt, mode) {
                // 形参类型已知时必须过 `effective_mode`：Copy 标量恒为 `Value`
                // （`int` 形参不能生成 `&i32`）。此前缺这一步，接口方法收标量形参
                // 时实参被加上 `&`，产物 `f.call(&n)` 对 `x: i32` 报 E0308。
                (Some(pt), Some(mode)) => {
                    let mode = self.effective_mode(mode, pt);
                    arg_strs.push(self.gen_mode_arg(a, mode, pt, env));
                }
                (Some(pt), None) => arg_strs.push(self.gen_arg(a, pt, env)),
                (None, Some(mode)) => arg_strs.push(self.gen_mode_arg_unknown(a, mode, env)),
                _ => {
                    if unknown_external {
                        arg_strs.push(self.gen_default_ref_arg(a, env));
                    } else {
                        arg_strs.push(self.gen_expr(a, env, None));
                    }
                }
            }
        }

        if is_list && name == "get" && !arg_strs.is_empty() {
            let r = self.gen_expr(recv.as_ref().unwrap(), env, None);
            return format!("{}[{}]", r, arg_strs[0]);
        }

        let is_string = matches!(recv_ty, Some(Type::String));
        // String 方法的 Java → Rust 映射（阶段 D / JR-E21）。
        //
        // **只做语义等价的映射**（JR-G4：不得静默改变语义）：
        // - `length` / `isEmpty` / `equals` / `startsWith` / `endsWith` / `contains` /
        //   `toString` 两边语义一致，直接换名；
        // - `trim` → `trim()`（Rust 去 Unicode 空白，Java 去 <= U+0020；常见场景等价）；
        // - `toUpperCase` / `toLowerCase` → `to_uppercase` / `to_lowercase`（同为 Unicode 感知）。
        //
        // **不映射**（语义有实质差异，报 JR6004 并建议用 raw）：
        // - `substring(a, b)` / `charAt(i)`：Java 按 UTF-16 码元、Rust 按字节，非 ASCII 会 panic；
        // - `split` / `replace`：Java 收正则、Rust 收字面量；
        // - `indexOf`：Java 找不到返回 `-1`，Rust 的 `find` 返回 `Option<usize>`，语义不同。
        let string_map = match name {
            "length" => Some("len"),
            "isEmpty" => Some("is_empty"),
            "equals" => Some("eq"),
            "startsWith" => Some("starts_with"),
            "endsWith" => Some("ends_with"),
            "contains" => Some("contains"),
            "toUpperCase" => Some("to_uppercase"),
            "toLowerCase" => Some("to_lowercase"),
            "trim" => Some("trim"),
            "toString" => Some("to_string"),
            _ => None,
        };
        let mapped = match (is_list, is_string, name) {
            (true, _, "add") => "push",
            (true, _, "size") => "len",
            (_, true, _) if string_map.is_some() => string_map.unwrap(),
            (_, _, "length") => "len",
            _ => name,
        };

        let call = match recv {
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
        };
        // `len()` 是 `usize`：Java 里 `size()` 返回 int，落回 int/long 上下文时补转换。
        if mapped == "len" {
            // String 的 `length()` 在 Java 里数的是**字符**，Rust 的 `len()` 是**字节**。
            // 非 ASCII 时两者不同（`"你好".length()` Java = 2、Rust len = 6），必须补
            // `chars().count()` 才能与解释器 / Java 语义一致（JR-G4 不得静默改变语义）。
            // 集合的 `size()` 两边都是元素个数，不需要转换。
            let call = if is_string {
                // `call` 形如 `s.len()`，去掉整个 `.len()` 后缀再拼 `.chars().count()`。
                let recv_expr = call.strip_suffix(".len()").unwrap_or(&call);
                format!("{}.chars().count()", recv_expr)
            } else {
                call
            };
            if matches!(expect, Some(Type::Int)) {
                return format!("({}) as i32", call);
            }
            if matches!(expect, Some(Type::Long)) {
                return format!("({}) as i64", call);
            }
            return call;
        }
        // 借用返回（`-> &T`）落到 owned 上下文时做落地转换。
        if let Some(exp) = expect {
            if !matches!(exp, Type::Void)
                && sig.as_ref().map_or(false, |s| s.1.is_borrow())
            {
                return self.owned_from_borrow(call, exp);
            }
        }
        call
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
                    self.iface_decls
                        .get(c)?
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
            Expr::Ident(n) => env.vars.lookup(n).cloned(),
            Expr::This(_) => env.self_class.clone().map(Type::Named),
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
                    // 接口静态方法 `Iface.method(..)`（JR-D10）
                    if let Some(iface) = self.iface_decls.get(en) {
                        if let Some(m) =
                            iface.methods.iter().find(|m| &m.name == name && m.is_static)
                        {
                            return Some(m.ret.clone());
                        }
                    }
                }
                // 迭代器链 `…collectList()` → List<元素类型>（JR-E18）
                if name == "collectList" {
                    if let Some(r) = recv.as_deref() {
                        if let Some(Type::List(inner)) =
                            self.infer_type(Self::chain_base(r), env)
                        {
                            return Some(Type::List(inner));
                        }
                    }
                    return None;
                }
                let recv_ty = recv.as_deref().and_then(|r| self.infer_type(r, env));
                // `list.get(i)` 在产物里是下标访问，类型就是元素类型（→ `v[i]`）。
                if name.as_str() == "get" {
                    match &recv_ty {
                        Some(Type::List(inner) | Type::Array(inner)) => {
                            return Some((**inner).clone())
                        }
                        Some(Type::Map(_, v)) => return Some((**v).clone()),
                        _ => {}
                    }
                }
                if let Some(Type::Map(_, v)) = &recv_ty {
                    if name.as_str() == "getOrNull" {
                        return Some(Type::Nullable(v.clone()));
                    }
                }
                // String 方法的返回类型（阶段 D / JR-E21）：链式调用 `s.trim().toLowerCase()`
                // 需要知道 `trim()` 返回 `&str`，否则外层方法映射不生效。
                if matches!(recv_ty, Some(Type::String)) {
                    match name.as_str() {
                        "length" => return Some(Type::Int),
                        "isEmpty" | "equals" | "startsWith" | "endsWith" | "contains" => {
                            return Some(Type::Boolean)
                        }
                        "trim" | "toUpperCase" | "toLowerCase" | "toString" => {
                            return Some(Type::String)
                        }
                        _ => {}
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
                        if let Some(iface) = self.iface_decls.get(&c) {
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
            Expr::Paren(inner) | Expr::Try(inner) | Expr::Await(inner) => {
                self.infer_type(inner, env)
            }
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
/// 递归展开 `interface A extends B, C`（JR-D6）：收集实现某接口时需要一并 impl 的祖先接口。
/// 按「先自己、再父接口」的顺序去重加入，保证子 trait 的 impl 先输出。
fn collect_ifaces(t: Type, prog: &Program, out: &mut Vec<Type>, seen: &mut HashSet<String>) {
    let n = type_name(&t).to_string();
    if n.is_empty() || !seen.insert(n.clone()) {
        return;
    }
    out.push(t);
    if let Some(iface) = prog.interfaces.iter().find(|x| x.name == n) {
        for p in &iface.extends {
            collect_ifaces(p.clone(), prog, out, seen);
        }
    }
}

fn type_name(ty: &Type) -> &str {
    match ty {
        Type::Named(n) | Type::Generic(n, _) => n,
        _ => "",
    }
}

/// 在已声明类型里找拼写最接近的名字（阶段 B / JR6001）。
///
/// 用 Levenshtein 距离，阈值取 `min(2, len/3)`：短名字（`Op`）容 1 处差异，
/// 长名字（`ConfigManager`）容 2 处。超过阈值返回 `None` —— 此时宁可判「可能来自外部
/// crate」给 warning，也不要武断地说「你是不是想写 X」。
///
/// 大小写差异按 1 处算（`Intger` vs `Integer` 距离 2 也是常见拼错形态）。
fn closest_known(name: &str, known: &HashSet<String>) -> Option<String> {
    let limit = std::cmp::min(2, std::cmp::max(1, name.chars().count() / 3));
    let mut best: Option<(usize, String)> = None;
    for cand in known {
        // 完全相同不是「拼错」，跳过。
        if cand == name {
            continue;
        }
        let d = levenshtein(name, cand);
        if d <= limit && best.as_ref().map_or(true, |(bd, _)| d < *bd) {
            best = Some((d, cand.clone()));
        }
    }
    best.map(|(_, s)| s)
}

/// 标准 Levenshtein 距离（字符级，非字节级——中文标识符也不会被切坏）。
fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.is_empty() {
        return b.len();
    }
    if b.is_empty() {
        return a.len();
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for i in 1..=a.len() {
        cur[0] = i;
        for j in 1..=b.len() {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            cur[j] = std::cmp::min(
                std::cmp::min(cur[j - 1] + 1, prev[j] + 1),
                prev[j - 1] + cost,
            );
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// 在某类型（含实现/继承的接口）的方法名里找拼写最接近的（阶段 B / JR6004）。
///
/// 阈值与 `closest_known` 一致。找不到返回 `None` —— 此时提示里就不猜「你是不是想写 X」。
fn closest_method(
    name: &str,
    owner: &str,
    classes: &HashMap<String, &ClassDecl>,
    ifaces: &HashMap<String, &InterfaceDecl>,
) -> Option<String> {
    let mut names: Vec<String> = Vec::new();
    if let Some(cd) = classes.get(owner) {
        names.extend(cd.methods.iter().map(|m| m.name.clone()));
        for imp in &cd.implements {
            collect_iface_method_names(type_name(imp), ifaces, &mut names, &mut HashSet::new());
        }
    }
    collect_iface_method_names(owner, ifaces, &mut names, &mut HashSet::new());

    let limit = std::cmp::min(2, std::cmp::max(1, name.chars().count() / 3));
    let mut best: Option<(usize, String)> = None;
    for cand in names {
        if cand == name {
            continue;
        }
        let d = levenshtein(name, &cand);
        if d <= limit && best.as_ref().map_or(true, |(bd, _)| d < *bd) {
            best = Some((d, cand));
        }
    }
    best.map(|(_, s)| s)
}

/// 递归收集接口（含 `extends` 链）的方法名。
fn collect_iface_method_names(
    iface: &str,
    ifaces: &HashMap<String, &InterfaceDecl>,
    out: &mut Vec<String>,
    seen: &mut HashSet<String>,
) {
    if !seen.insert(iface.to_string()) {
        return;
    }
    let Some(d) = ifaces.get(iface) else {
        return;
    };
    out.extend(d.methods.iter().map(|m| m.name.clone()));
    for p in &d.extends {
        collect_iface_method_names(type_name(p), ifaces, out, seen);
    }
}

/// 在类型里找第一个「未知的 `Named`」（阶段 B / JR6001）。
///
/// 只认 `Type::Named`：`Generic(name, args)` 的名字部分是用户写的类/接口名（要查），
/// 但实参递归检查（`List<Intger>` 也要抓）。标量与内建集合在 parser 阶段就变成了
/// 对应的 `Type` 变体，不会走到这里。
fn unknown_named_in(
    ty: &Type,
    known: &HashSet<String>,
    external: &HashSet<String>,
) -> Option<String> {
    match ty {
        Type::Named(n) => {
            if known.contains(n) || external.contains(n) {
                None
            } else {
                Some(n.clone())
            }
        }
        Type::Generic(n, args) => {
            if !known.contains(n) && !external.contains(n) {
                return Some(n.clone());
            }
            args.iter().find_map(|a| unknown_named_in(a, known, external))
        }
        Type::List(inner) | Type::Set(inner) | Type::Array(inner) | Type::Nullable(inner) => {
            unknown_named_in(inner, known, external)
        }
        Type::Map(k, v) => {
            unknown_named_in(k, known, external).or_else(|| unknown_named_in(v, known, external))
        }
        _ => None,
    }
}

/// 字面量能否自适应到目标标量类型（Rust 的字面量类型由上下文推断）。
///
/// - 整数字面量 → 任意整数目标：可以（`let m: i64 = 5;` 合法）。
/// - 浮点字面量 → 任意浮点目标：可以（`let f: f32 = 1.5;` 合法）。
/// - 整数字面量 → 浮点目标：**不可以**（`let d: f64 = 1;` 报 E0308，阶段 0 已补 `.0`）。
/// - 浮点字面量 → 整数目标：不可以。
fn literal_fits(e: &Expr, to: &Type) -> bool {
    let to_float = matches!(to, Type::Double | Type::Float)
        || matches!(to, Type::RustPrim(n) if n == "f32" || n == "f64");
    let to_int = matches!(to, Type::Int | Type::Long | Type::Char)
        || matches!(to, Type::RustPrim(n) if n.starts_with('i') || n.starts_with('u'));
    match strip_parens_expr(e) {
        Expr::Int(_) => to_int,
        Expr::Float(_) => to_float,
        _ => false,
    }
}

/// owner 是否内置类型名（白名单报错时用来换提示措辞）。
fn builtin_kind_name(owner: &str) -> Option<&'static str> {
    match owner {
        "String" | "List" | "Map" | "Set" => Some(match owner {
            "String" => "String",
            "List" => "List",
            "Map" => "Map",
            _ => "Set",
        }),
        _ => None,
    }
}

/// 内置类型支持的方法列表（拼成提示文本）。
fn builtin_method_list(kind: &str) -> String {
    let names: &[&str] = match kind {
        "String" => &[
            "length", "isEmpty", "equals", "startsWith", "endsWith", "contains",
            "toUpperCase", "toLowerCase", "trim", "toString",
        ],
        "List" => &["add", "size", "get", "isEmpty", "stream", "mutable", "length"],
        "Map" => &["put", "get", "getOrNull", "size", "containsKey", "isEmpty"],
        _ => &["add", "contains", "remove", "size", "isEmpty"],
    };
    names
        .iter()
        .map(|n| format!("`{}`", n))
        .collect::<Vec<_>>()
        .join(" / ")
}

/// 内置类型的种类（阶段 D 的白名单键）。
fn builtin_kind(t: &Type) -> Option<&'static str> {
    match t {
        Type::String => Some("String"),
        Type::List(_) | Type::Array(_) => Some("List"),
        Type::Map(..) => Some("Map"),
        Type::Set(_) => Some("Set"),
        _ => None,
    }
}

/// 内置类型的已知方法白名单（阶段 D / JR-E21）。
///
/// **只列真正被映射或有语义等价实现的方法**。名单外的报 `JR6004` 并建议用 `raw`。
/// 这张表必须与 `gen_call` 里的映射表、以及 `infer_type` 的返回类型表保持一致——
/// 三处不同步就会出现「检查放行但产物编译不过」或「检查报错但产物其实能用」。
fn builtin_method_known(kind: &str, name: &str) -> bool {
    let names: &[&str] = match kind {
        "String" => &[
            "length",
            "isEmpty",
            "equals",
            "startsWith",
            "endsWith",
            "contains",
            "toUpperCase",
            "toLowerCase",
            "trim",
            "toString",
        ],
        "List" => &[
            "add",
            "size",
            "get",
            "isEmpty",
            "stream",
            "mutable",
            "length",
        ],
        "Map" => &["put", "get", "getOrNull", "size", "containsKey", "isEmpty"],
        "Set" => &["add", "contains", "remove", "size", "isEmpty"],
        _ => return true,
    };
    names.contains(&name)
}

/// 类型的名字（`Named` / `Generic` 的名字部分），其余返回 `None`。
fn type_name_opt(ty: &Type) -> Option<&str> {
    match ty {
        Type::Named(n) | Type::Generic(n, _) => Some(n.as_str()),
        _ => None,
    }
}

/// 是否是泛型参数名（单字母大写是最常见的写法，但也可能是 `TValue`）。
///
/// 判定保守：只把「单字符大写」或「以 T 开头且全大写字母/数字」当泛型参数。
/// 误判成泛型参数的代价是漏报一次赋值检查（无害），误判成类的代价是误报（有害），
/// 所以这里偏向「宁可当泛型」。
fn is_generic_param(n: &str) -> bool {
    if n.chars().count() == 1 {
        return n.chars().next().is_some_and(|c| c.is_ascii_uppercase());
    }
    n.starts_with('T') && n.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
}

/// 递归收集赋值 / 返回的类型不匹配（阶段 C / JR6007 / JR6008）。
///
/// 边扫描边把 `Stmt::Let` 的声明写进 `env`，保证后续语句能解析变量类型。
/// `raw` 块跳过（JR-X3）。
fn collect_assign_mismatches(
    stmts: &[Stmt],
    env: &mut Env,
    g: &Codegen,
    bad: &mut Vec<(usize, String, String, &'static str)>,
) {
    for s in stmts {
        match s {
            Stmt::Raw { .. } => {}
            Stmt::Let { ty, name, init, line } => {
                if let Some(from) = g.scalar_source_type(init, env) {
                    // 字面量可以自适应：`float f = 1.5;` 里的 `1.5` 在 Rust 里
                    // 会推断成 f32，所以「浮点字面量 → f32」不算窄化。
                    if !g.assignable(&from, ty) && !literal_fits(init, ty) {
                        bad.push((
                            *line,
                            g.rust_type(&from),
                            g.rust_type(ty),
                            "assign",
                        ));
                    }
                }
                env.vars.declare(name.clone(), ty.clone());
            }
            Stmt::Assign { target, value, line } => {
                // 只检查目标类型能静态确定的赋值（局部变量 / `this.字段`）。
                let target_ty = match strip_parens_expr(target) {
                    Expr::Ident(n) => env.vars.lookup(n).cloned(),
                    Expr::Field(base, f) if matches!(strip_parens_expr(base), Expr::This(_)) => {
                        env.self_class.as_ref().and_then(|cn| {
                            g.classes
                                .get(cn)
                                .and_then(|cd| cd.fields.iter().find(|x| &x.name == f))
                                .map(|x| x.ty.clone())
                        })
                    }
                    _ => None,
                };
                if let (Some(tt), Some(from)) = (target_ty, g.scalar_source_type(value, env)) {
                    if !g.assignable(&from, &tt) {
                        bad.push((*line, g.rust_type(&from), g.rust_type(&tt), "assign"));
                    }
                }
            }
            Stmt::Return { value: Some(e), line } => {
                let ret = env.ret.clone();
                if let Some(from) = g.scalar_source_type(e, env) {
                    if !g.assignable(&from, &ret) {
                        bad.push((*line, g.rust_type(&from), g.rust_type(&ret), "return"));
                    }
                }
            }
            Stmt::If { then, els, .. } => {
                collect_assign_mismatches(then, env, g, bad);
                if let Some(e) = els {
                    collect_assign_mismatches(e, env, g, bad);
                }
            }
            Stmt::While { body, .. }
            | Stmt::ForEach { body, .. }
            | Stmt::ForRange { body, .. }
            | Stmt::Sync { body, .. }
            | Stmt::Spawn { body, .. } => collect_assign_mismatches(body, env, g, bad),
            _ => {}
        }
    }
}

/// 递归收集「条件不是 bool」的 `if` / `while`（阶段 B / JR6009）。
///
/// 边扫描边把 `Stmt::Let` 的声明写进 `env`，这样 `int n = 1; if (n)` 里的
/// `n` 才能解析出 `int`。`raw` 块跳过（JR-X3）。
fn collect_non_bool_conds(
    stmts: &[Stmt],
    env: &mut Env,
    g: &Codegen,
    bad: &mut Vec<(usize, String)>,
) {
    for s in stmts {
        match s {
            Stmt::Raw { .. } => {}
            Stmt::Let { ty, name, init, line } => {
                let _ = line;
                // 初始化器里也可能有条件表达式（如闭包体），但 jrust 的闭包体只支持
                // 单表达式，不含语句，故这里只关心声明本身。
                let _ = init;
                env.vars.declare(name.clone(), ty.clone());
            }
            Stmt::If { cond, then, els, line } => {
                if let Some(tn) = non_bool_cond_type(cond, env, g) {
                    bad.push((*line, tn));
                }
                collect_non_bool_conds(then, env, g, bad);
                if let Some(e) = els {
                    collect_non_bool_conds(e, env, g, bad);
                }
            }
            Stmt::While { cond, body, line } => {
                if let Some(tn) = non_bool_cond_type(cond, env, g) {
                    bad.push((*line, tn));
                }
                collect_non_bool_conds(body, env, g, bad);
            }
            Stmt::ForEach { body, .. } | Stmt::ForRange { body, .. } => {
                // `for` 的迭代器/区间由语法限定，不会是非 bool 条件。
                collect_non_bool_conds(body, env, g, bad);
            }
            Stmt::Sync { body, .. } | Stmt::Spawn { body, .. } => {
                collect_non_bool_conds(body, env, g, bad)
            }
            _ => {}
        }
    }
}

/// 条件表达式的类型若可确定为「非 bool」，返回其类型名。
///
/// 比较 / 逻辑运算（`<` / `==` / `&&` / `!`）天然产 bool，直接放行；
/// 其余交给 `infer_type`，拿不到就放行（无法判定不猜）。
fn non_bool_cond_type(cond: &Expr, env: &Env, g: &Codegen) -> Option<String> {
    let ty = match strip_parens_expr(cond) {
        Expr::Bool(_) => return None,
        Expr::Binary { op, .. } => match op {
            BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge | BinOp::Eq | BinOp::Ne | BinOp::And
            | BinOp::Or => return None,
            _ => g.infer_type(cond, env)?,
        },
        Expr::Unary { op, .. } if matches!(op, UnOp::Not) => return None,
        _ => g.infer_type(cond, env)?,
    };
    // `bool` 本身是合法条件——必须排除，否则 `if (flag)` 会被误报。
    // 用 `rust_type` 判而不是 `matches!(ty, Type::Boolean)`，因为用户直接写
    // Rust 原生类型名 `bool` 时走的是 `Type::RustPrim("bool")`。
    if g.rust_type(&ty) == "bool" {
        return None;
    }
    Some(g.rust_type(&ty))
}

/// 递归收集语句里的局部变量声明类型（阶段 B / JR6001）。
///
/// `Stmt::Let` 是 `var x = 1;` / `Intger x = 1;` 的落点；其余语句递归下去，
/// 但**跳过 `Stmt::Raw`**（JR-X3：raw 不参与 jrust 诊断）。
fn collect_local_types(
    stmts: &[Stmt],
    bad: &mut Vec<(usize, String)>,
    known: &HashSet<String>,
    external: &HashSet<String>,
) {
    for s in stmts {
        match s {
            Stmt::Let { ty, line, .. } => {
                if let Some(n) = unknown_named_in(ty, known, external) {
                    bad.push((*line, n));
                }
            }
            Stmt::If { then, els, .. } => {
                collect_local_types(then, bad, known, external);
                if let Some(e) = els {
                    collect_local_types(e, bad, known, external);
                }
            }
            Stmt::While { body, .. }
            | Stmt::ForEach { body, .. }
            | Stmt::ForRange { body, .. }
            | Stmt::Sync { body, .. }
            | Stmt::Spawn { body, .. } => collect_local_types(body, bad, known, external),
            Stmt::Raw { .. } => {}
            _ => {}
        }
    }
}

/// 把类型里的类型参数（`T`）替换成实参类型（`Integer` → `Int`）。
fn subst_type(ty: &Type, map: &HashMap<String, Type>) -> Type {    match ty {
        Type::Named(n) => map.get(n).cloned().unwrap_or_else(|| ty.clone()),
        Type::Generic(n, args) => {
            Type::Generic(n.clone(), args.iter().map(|a| subst_type(a, map)).collect())
        }
        Type::List(inner) => Type::List(Box::new(subst_type(inner, map))),
        Type::Map(k, v) => Type::Map(
            Box::new(subst_type(k, map)),
            Box::new(subst_type(v, map)),
        ),
        Type::Set(inner) => Type::Set(Box::new(subst_type(inner, map))),
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

/// 把 `"..."` 形式的 token 文本还原成字符串内容（`@raw("…")` 用）。
fn strip_str_literal(s: &str) -> Option<String> {
    let inner = s.trim().strip_prefix('"')?.strip_suffix('"')?;
    Some(inner.replace("\\\"", "\"").replace("\\\\", "\\"))
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
        Type::List(inner) | Type::Array(inner) | Type::Set(inner) | Type::Nullable(inner) => {
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
        Type::List(inner) | Type::Array(inner) | Type::Set(inner) => {
            type_cloneable(inner, named, params)
        }
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

/// Rust 2018+ 的关键字里，能靠 `r#` 恢复成标识符的那些（SPEC JR-L1）。
///
/// 不含 `crate` / `self` / `Self` / `super` / `_`：这几个**不能**写 `r#`，
/// 撞上只能报诊断让用户改名。`dyn` 在 2015 是弱关键字、2018 起是关键字，这里按 2018 处理。
const RUST_KEYWORDS: &[&str] = &[
    "as", "break", "const", "continue", "dyn", "else", "enum", "extern", "false", "fn", "for",
    "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return",
    "static", "struct", "trait", "true", "type", "unsafe", "use", "where", "while", "async",
    "await", "box", "do", "final", "macro", "override", "priv", "try", "typeof", "unsized",
    "virtual", "yield",
];

/// 不能用 `r#` 挽救的关键字（写 `r#crate` 本身非法）。
const RUST_KEYWORDS_NO_RAW: &[&str] = &["crate", "self", "Self", "super", "_"];

/// 把 jrust 标识符渲染成合法的 Rust 标识符：撞关键字时加 `r#`（JR-L1）。
///
/// 只处理**标识符位置**（类型名 / 模块名 / 变量名 / 方法名）。`r#` 不能用于
/// `crate` / `self` / `Self` / `super`，那几个由调用方用 `keyword_conflict()` 报诊断。
pub fn rust_ident(name: &str) -> String {
    if RUST_KEYWORDS.contains(&name) {
        format!("r#{}", name)
    } else {
        name.to_string()
    }
}

/// 这个标识符是否「无法用 `r#` 挽救」的 Rust 关键字（`crate` / `self` / `Self` / `super`）。
fn keyword_conflict(name: &str) -> bool {
    RUST_KEYWORDS_NO_RAW.contains(&name)
}

/// `keyword_conflict` 的公开别名：`main.rs` 的 package 校验也要用同一判据。
pub fn is_keyword_unrecoverable(name: &str) -> bool {
    keyword_conflict(name)
}

/// 内置映射类型（不是「外部 crate」）：它们的调用由 `gen_call` 特化处理，
/// 不属于 `JR4101`（未知签名）的适用范围。
fn is_known_builtin_type(name: &str) -> bool {
    matches!(
        name,
        "String"
            | "List"
            | "ArrayList"
            | "LinkedList"
            | "Vector"
            | "Map"
            | "HashMap"
            | "Set"
            | "HashSet"
    )
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

// ---------- spawn 自由变量（Stage E / JR-C11） ----------

/// `spawn` 块里引用到的、但在块内**声明**的外层变量（自由变量），按出现顺序。
///
/// `async move` 会把这些变量整个搬进闭包，外层再用就 `E0382`。Stage E 的做法是
/// 在 `spawn` **之前**给每个自由变量生成 `let __jr_sp<N>_x = x.clone();`，
/// 闭包内再 `let x = __jr_sp<N>_x;` 接回名字 —— 闭包持有副本、外层保留原值，
/// 与 Java 里「lambda 捕获的局部变量是副本」的直觉一致。
/// 注意不能用同名 shadowing（`let x = x.clone();`）：那只是给同一块内存起第二个名字，
/// `async move` 照样把外层 `x` 移走。
fn spawn_free_vars(body: &[Stmt], outer: &[String]) -> Vec<String> {
    // 块内声明过的名字（含循环变量）：它们是闭包自己的，不算自由变量。
    let mut declared: HashSet<String> = HashSet::new();
    each_stmt(body, &mut |s| match s {
        Stmt::Let { name, .. } => {
            declared.insert(name.clone());
        }
        Stmt::ForEach { name, .. } | Stmt::ForRange { name, .. } => {
            declared.insert(name.clone());
        }
        _ => {}
    });

    let mut out: Vec<String> = Vec::new();
    each_stmt_expr(body, &mut |e| {
        // `each_expr` 会递归进 `x.f` / `x[i]` / `x.m()`，所以只看裸 `Ident` 就能拿到根变量。
        if let Expr::Ident(n) = e {
            if outer.iter().any(|o| o == n)
                && !declared.contains(n)
                && !out.iter().any(|x| x == n)
            {
                out.push(n.clone());
            }
        }
    });
    out
}

// ---------- @shared 分析（JR-C13） ----------

/// `@shared` 字段。
fn field_is_shared(f: &Field) -> bool {
    f.attrs.iter().any(|a| a.name == "shared")
}

/// 赋值目标是否落在 `this.<field>` 上（含 `this.f.x = ..` / `this.f[i] = ..`）。
fn target_writes_this_field(target: &Expr) -> Option<String> {
    match target {
        Expr::Field(recv, name) => match &**recv {
            Expr::This(_) => Some(name.clone()),
            inner => target_writes_this_field(inner),
        },
        Expr::Index(recv, _) => target_writes_this_field(recv),
        _ => None,
    }
}

/// 遍历语句（递归进入子块）。
fn each_stmt<'a>(stmts: &'a [Stmt], f: &mut dyn FnMut(&'a Stmt)) {
    for s in stmts {
        f(s);
        match s {
            Stmt::If { then, els, .. } => {
                each_stmt(then, f);
                if let Some(e) = els {
                    each_stmt(e, f);
                }
            }
            Stmt::While { body, .. }
            | Stmt::ForEach { body, .. }
            | Stmt::ForRange { body, .. }
            | Stmt::Sync { body, .. }
            | Stmt::Spawn { body, .. } => each_stmt(body, f),
            Stmt::Switch { cases, default, .. } => {
                for c in cases {
                    each_stmt(&c.body, f);
                }
                if let Some(d) = default {
                    each_stmt(d, f);
                }
            }
            _ => {}
        }
    }
}

/// 遍历表达式（递归进入子表达式）。
fn each_expr<'a>(e: &'a Expr, f: &mut dyn FnMut(&'a Expr)) {
    f(e);
    match e {
        Expr::Field(r, _)
        | Expr::Paren(r)
        | Expr::Await(r)
        | Expr::Try(r)
        | Expr::Unary { expr: r, .. } => each_expr(r, f),
        Expr::Index(a, b) | Expr::Binary { lhs: a, rhs: b, .. } => {
            each_expr(a, f);
            each_expr(b, f);
        }
        Expr::Call { recv, args, .. } => {
            if let Some(r) = recv {
                each_expr(r, f);
            }
            for a in args {
                each_expr(a, f);
            }
        }
        Expr::New { args, .. } => {
            for a in args {
                each_expr(a, f);
            }
        }
        Expr::NewArray { len, .. } => each_expr(len, f),
        Expr::Closure { body, .. } => each_expr(body, f),
        _ => {}
    }
}

/// 遍历语句里直接出现的表达式（子块由 `each_stmt` 负责递归）。
fn each_stmt_expr<'a>(stmts: &'a [Stmt], f: &mut dyn FnMut(&'a Expr)) {
    each_stmt(stmts, &mut |s| match s {
        Stmt::Let { init, .. } => each_expr(init, f),
        Stmt::Assign { target, value, .. } => {
            each_expr(target, f);
            each_expr(value, f);
        }
        Stmt::Expr { expr, .. } => each_expr(expr, f),
        Stmt::Return { value: Some(e), .. } => each_expr(e, f),
        Stmt::If { cond, .. } => each_expr(cond, f),
        Stmt::While { cond, .. } => each_expr(cond, f),
        Stmt::ForEach { iter, .. } => each_expr(iter, f),
        Stmt::ForRange { start, end, .. } => {
            each_expr(start, f);
            each_expr(end, f);
        }
        Stmt::Switch {
            subject,
            cases,
            ..
        } => {
            each_expr(subject, f);
            for c in cases {
                if let Some(g) = &c.guard {
                    each_expr(g, f);
                }
            }
        }
        Stmt::Throw { expr, .. } => each_expr(expr, f),
        Stmt::Sync { lock, .. } => each_expr(lock, f),
        _ => {}
    });
}

/// 方法体里是否直接写了 `this.<field>`。
fn writes_this_field(stmts: &[Stmt], field: &str) -> bool {
    let mut hit = false;
    each_stmt(stmts, &mut |s| {
        if let Stmt::Assign { target, .. } = s {
            if target_writes_this_field(target).as_deref() == Some(field) {
                hit = true;
            }
        }
    });
    hit
}

/// 方法体里是否写了任意 `this.<x>`（用于判断「调用它会可变借用」）。
fn writes_this(stmts: &[Stmt]) -> bool {
    let mut hit = false;
    each_stmt(stmts, &mut |s| {
        if let Stmt::Assign { target, .. } = s {
            if target_writes_this_field(target).is_some() {
                hit = true;
            }
        }
    });
    hit
}

/// 方法体里是否对 `this.<field>` 调用了会改自身状态的方法。
fn calls_mut_on_field(
    stmts: &[Stmt],
    field: &str,
    classes: &HashMap<String, &ClassDecl>,
    ty: &Type,
) -> bool {
    let class_name = match ty {
        Type::Named(n) | Type::Generic(n, _) => n.clone(),
        _ => return false,
    };
    let Some(cd) = classes.get(class_name.as_str()) else {
        return false;
    };
    let mut hit = false;
    each_stmt_expr(stmts, &mut |e| {
        if let Expr::Call {
            recv: Some(r),
            name,
            ..
        } = e
        {
            if matches!(&**r, Expr::Field(base, f) if f == field && matches!(**base, Expr::This(_))) {
                if let Some(m) = cd
                    .methods
                    .iter()
                    .find(|m| &m.name == name && !m.is_static)
                {
                    if writes_this(&m.body) {
                        hit = true;
                    }
                }
            }
        }
    });
    hit
}

/// 一个类里需要 `Arc<Mutex<T>>` 的 `@shared` 字段（构造器赋值不算写）。
fn shared_mut_fields(
    class: &ClassDecl,
    classes: &HashMap<String, &ClassDecl>,
) -> HashSet<String> {
    let mut out = HashSet::new();
    for f in class.fields.iter().filter(|f| field_is_shared(f)) {
        let mutated = class.methods.iter().any(|m| {
            writes_this_field(&m.body, &f.name) || calls_mut_on_field(&m.body, &f.name, classes, &f.ty)
        });
        if mutated {
            out.insert(f.name.clone());
        }
    }
    out
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
            Stmt::Sync { body, .. } => walk_mut(body, set),
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
                "add" | "push" | "insert" | "remove" | "clear" | "set" | "put" | "mutable"
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
        Stmt::Spawn { body, .. } => stmts_use_ident(body, want),
        Stmt::Sync { lock, body, .. } => {
            expr_uses_ident(lock, want) || stmts_use_ident(body, want)
        }
        Stmt::Raw { .. } => false,
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

/// 统计 raw 块（JR-X3）：返回 `(块数, 内容行数)`。
///
/// `raw` 内容原样内联、不参与 jrust 语义分析，因此不计入诊断覆盖率；`jrust verify`
/// 用这组数字在报告里单列。覆盖类级 `raw` 成员、构造器 / 方法体、接口默认方法体；
/// 单行 `raw { ... }` 按 1 行计。
pub fn raw_usage(prog: &Program) -> (usize, usize) {
    let mut blocks = 0usize;
    let mut lines = 0usize;
    let mut add = |code: &str| {
        blocks += 1;
        lines += code.lines().count().max(1);
    };
    for c in &prog.classes {
        for r in &c.raw_members {
            add(&r.code);
        }
        if let Some(ctor) = &c.ctor {
            scan_body_raw(&ctor.body, &mut add);
        }
        for m in &c.methods {
            scan_body_raw(&m.body, &mut add);
        }
    }
    for i in &prog.interfaces {
        for m in &i.methods {
            if let Some(body) = &m.body {
                scan_body_raw(body, &mut add);
            }
        }
    }
    (blocks, lines)
}

fn scan_body_raw(body: &[Stmt], add: &mut impl FnMut(&str)) {
    each_stmt(body, &mut |s| {
        if let Stmt::Raw { code, .. } = s {
            add(code);
        }
    });
    each_stmt_expr(body, &mut |e| scan_expr_raw(e, add));
}

fn scan_expr_raw(e: &Expr, add: &mut impl FnMut(&str)) {
    each_expr(e, &mut |e| {
        if let Expr::Raw { code, .. } = e {
            add(code);
        }
    });
}
