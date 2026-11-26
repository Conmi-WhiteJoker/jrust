//! 手写递归下降语法分析器。
//!
//! 设计原则：只接受 jrust 支持的 Java 子集；遇到被砍掉的特性（继承、try/catch、
//! instanceof、synchronized、注解……）时给出「为什么 Rust 不这么干」的解释，
//! 而不是含糊地报「语法错误」。`null` 只允许出现在可空类型 `T?` 的上下文。

use crate::ast::*;
use crate::diag::Diag;
use crate::lexer::{lex, Kw, Tok, Token};

enum TopItem {
    Class(ClassDecl),
    Interface(InterfaceDecl),
    Enum(EnumDecl),
}

pub fn parse_file(src: &str) -> Result<Program, Vec<Diag>> {
    let toks = lex(src)?;
    let mut p = Parser {
        toks,
        pos: 0,
        diags: Vec::new(),
        class_name: String::new(),
    };
    let prog = p.parse_program();
    if p.diags.is_empty() {
        Ok(prog)
    } else {
        Err(p.diags)
    }
}

struct Parser {
    toks: Vec<Token>,
    pos: usize,
    diags: Vec<Diag>,
    /// 当前正在解析的类名，用于识别构造器（方法名 == 类名）。
    class_name: String,
}

impl Parser {
    // ---------- token 操作 ----------

    fn peek(&self) -> &Tok {
        &self.toks[self.pos].tok
    }

    fn peek_at(&self, n: usize) -> &Tok {
        self.toks
            .get(self.pos + n)
            .map(|t| &t.tok)
            .unwrap_or(&Tok::Eof)
    }

    fn line(&self) -> usize {
        self.toks[self.pos].line
    }

    fn at_eof(&self) -> bool {
        matches!(self.peek(), Tok::Eof)
    }

    fn bump(&mut self) -> Tok {
        let t = self.toks[self.pos].tok.clone();
        if self.pos + 1 < self.toks.len() {
            self.pos += 1;
        }
        t
    }

    fn check_punct(&self, p: &str) -> bool {
        matches!(self.peek(), Tok::Punct(x) if *x == p)
    }

    fn check_kw(&self, k: Kw) -> bool {
        matches!(self.peek(), Tok::Kw(x) if *x == k)
    }

    fn eat_punct(&mut self, p: &str) -> bool {
        if self.check_punct(p) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn eat_kw(&mut self, k: Kw) -> bool {
        if self.check_kw(k) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn expect_punct(&mut self, p: &str) -> bool {
        if self.eat_punct(p) {
            true
        } else {
            let d = self.describe();
            self.err(format!("这里期望 `{}`，实际是 {}", p, d));
            false
        }
    }

    fn expect_ident(&mut self) -> String {
        if let Tok::Ident(s) = self.peek().clone() {
            self.bump();
            s
        } else {
            let d = self.describe();
            self.err(format!("这里期望标识符，实际是 {}", d));
            self.bump();
            String::new()
        }
    }

    fn err(&mut self, msg: impl Into<String>) {
        let line = self.line();
        self.diags.push(Diag::error(line, msg));
    }

    fn err_hint(&mut self, msg: impl Into<String>, hint: impl Into<String>) {
        let line = self.line();
        self.diags.push(Diag::error(line, msg).with_hint(hint));
    }

    fn describe(&self) -> String {
        match self.peek() {
            Tok::Ident(s) => format!("标识符 `{}`", s),
            Tok::Int(n) => format!("整数 `{}`", n),
            Tok::Float(f) => format!("浮点数 `{}`", f),
            Tok::Str(_) => "字符串字面量".to_string(),
            Tok::CharLit(_) => "字符字面量".to_string(),
            Tok::Kw(k) => format!("关键字 `{}`", kw_text(*k)),
            Tok::Punct(p) => format!("`{}`", p),
            Tok::Raw(_) => "raw 块".to_string(),
            Tok::Doc(_) => "文档注释".to_string(),
            Tok::Eof => "文件末尾".to_string(),
        }
    }

    // ---------- 跳过 / 错误恢复 ----------

    fn skip_parens(&mut self) {
        if !self.check_punct("(") {
            return;
        }
        let mut depth = 0i32;
        while !self.at_eof() {
            if self.check_punct("(") {
                depth += 1;
                self.bump();
            } else if self.check_punct(")") {
                depth -= 1;
                self.bump();
                if depth <= 0 {
                    break;
                }
            } else {
                self.bump();
            }
        }
    }

    fn skip_block(&mut self) {
        if !self.check_punct("{") {
            return;
        }
        let mut depth = 0i32;
        while !self.at_eof() {
            if self.check_punct("{") {
                depth += 1;
                self.bump();
            } else if self.check_punct("}") {
                depth -= 1;
                self.bump();
                if depth <= 0 {
                    break;
                }
            } else {
                self.bump();
            }
        }
    }

    fn skip_body(&mut self) {
        if self.check_punct("{") {
            self.skip_block();
        } else {
            self.skip_to_stmt_end();
        }
    }

    fn skip_to_stmt_end(&mut self) {
        while !self.at_eof() {
            if self.check_punct(";") {
                self.bump();
                return;
            }
            if self.check_punct("}") {
                return;
            }
            self.bump();
        }
    }

    fn skip_generic_args(&mut self) {
        if !self.check_punct("<") {
            return;
        }
        let mut depth = 0i32;
        while !self.at_eof() {
            if self.check_punct("<") {
                depth += 1;
                self.bump();
            } else if self.check_punct(">") {
                depth -= 1;
                self.bump();
                if depth <= 0 {
                    break;
                }
            } else {
                self.bump();
            }
        }
    }

    /// 收集紧邻的 `///` 文档注释（JR-L2），返回去掉 `///` 后的各行文本。
    fn parse_docs(&mut self) -> Vec<String> {
        let mut docs = Vec::new();
        while let Tok::Doc(s) = self.peek().clone() {
            docs.push(s);
            self.bump();
        }
        docs
    }

    /// 解析 `@Xxx(args)` 注解序列（JR-D9）。实参按 token 原文拼接，交给 codegen 输出属性。
    fn parse_annotations(&mut self) -> Vec<Attr> {
        self.parse_annotations_impl(false)
    }

    /// 形参位置的注解（Stage D）：额外接受 `@ref` / `@mut`。
    fn parse_param_annotations(&mut self) -> Vec<Attr> {
        self.parse_annotations_impl(true)
    }

    fn parse_annotations_impl(&mut self, allow_borrow: bool) -> Vec<Attr> {
        let mut attrs = Vec::new();
        while self.check_punct("@") {
            let aline = self.line();
            self.bump(); // @
            // `raw` 现在是关键字（JR-X1），但 `@raw("…")` 注解（JR-D9）仍然合法。
            if !matches!(self.peek(), Tok::Ident(_) | Tok::Kw(Kw::Raw)) {
                let d = self.describe();
                self.err(format!("`@` 后面期望注解名，实际是 {}", d));
                break;
            }
            let mut name = if self.eat_kw(Kw::Raw) {
                "raw".to_string()
            } else {
                self.expect_ident()
            };
            while self.eat_punct(".") {
                let seg = self.expect_ident();
                name.push('.');
                name.push_str(&seg);
            }
            let mut args = None;
            if self.eat_punct("(") {
                let mut depth = 1usize;
                let mut buf = String::new();
                while !self.at_eof() {
                    if self.check_punct("(") {
                        depth += 1;
                    } else if self.check_punct(")") {
                        depth -= 1;
                        if depth == 0 {
                            self.bump();
                            break;
                        }
                    }
                    buf.push_str(&tok_text(self.peek()));
                    self.bump();
                }
                if depth != 0 {
                    self.err("注解的括号没有闭合");
                }
                args = Some(buf);
            }
            if matches!(name.as_str(), "ref" | "mut") {
                if !allow_borrow {
                    self.err_hint(
                        format!("`@{}` 只能写在方法形参上", name),
                        "`@ref` / `@mut` 用来覆盖形参借用推断，写在方法签名或接口方法声明的形参前。",
                    );
                    continue;
                }
                if args.is_some() {
                    self.err_hint(
                        format!("`@{}` 不接受参数", name),
                        "直接写 `@ref` / `@mut`，括号里什么都不用填。",
                    );
                }
                if attrs.iter().any(|a: &Attr| a.name == "ref" || a.name == "mut") {
                    self.err_hint(
                        format!("形参上 `@ref` / `@mut` 只能出现一个（第 {} 行）", aline),
                        "两者互斥：只读借用写 `@ref`，可变借用写 `@mut`。",
                    );
                    continue;
                }
            }
            attrs.push(Attr { name, args });
        }
        attrs
    }

    // ---------- 顶层 ----------

    fn parse_program(&mut self) -> Program {
        let mut prog = Program::default();
        while !self.at_eof() {
            let start = self.pos;
            let docs = self.parse_docs();
            match self.peek().clone() {
                Tok::Kw(Kw::Import) | Tok::Kw(Kw::Package) => {
                    let is_package = self.check_kw(Kw::Package);
                    let line = self.line();
                    self.bump();
                    let mut path = String::new();
                    // 路径里出现关键字（如 `package move.thing;`）不能静默跳过：跳过会让
                    // 路径少一段，产物变成 `crate::::thing::Thing` 这种非法 Rust（JR-L1）。
                    let mut bad_kw: Option<String> = None;
                    while !self.at_eof() && !self.check_punct(";") {
                        match self.peek().clone() {
                            Tok::Ident(s) => {
                                path.push_str(&s);
                                self.bump();
                            }
                            Tok::Kw(k) => {
                                if bad_kw.is_none() {
                                    bad_kw = Some(kw_text(k).to_string());
                                }
                                self.bump();
                            }
                            Tok::Punct(".") => {
                                path.push('.');
                                self.bump();
                            }
                            Tok::Punct("*") => {
                                path.push('*');
                                self.bump();
                            }
                            _ => {
                                self.bump();
                            }
                        }
                    }
                    self.eat_punct(";");
                    if let Some(kw) = bad_kw {
                        self.diags.push(
                            Diag::error(
                                line,
                                format!(
                                    "`{}` 里的 `{}` 是 jrust / Rust 关键字，不能用作包名或类名",
                                    if is_package { "package" } else { "import" },
                                    kw
                                ),
                            )
                            .with_hint(
                                "换一个不含关键字的目录名 / 类名，例如 `app.thing` 而不是 `move.thing`。",
                            ),
                        );
                        continue;
                    }
                    if is_package {
                        // 多文件模式（JR-S4/M3）：`package a.b.c;` 声明模块路径。
                        if path.is_empty() {
                            self.diags.push(
                                Diag::error(line, "package 后面需要模块路径，例如 `package app.greet;`")
                                    .with_hint(
                                        "模块路径决定产物目录：`package app.greet;` → `app/greet.rs`。",
                                    ),
                            );
                        } else if prog.package.is_some() {
                            self.diags.push(
                                Diag::error(line, "一个文件只能有一个 package 声明").with_hint(
                                    "把多个 package 语句合并成一个；一个 .jrust 只对应一个模块。",
                                ),
                            );
                        } else {
                            prog.package = Some(path);
                        }
                    } else if !path.is_empty() {
                        prog.imports.push(path);
                    }
                }
                Tok::Kw(Kw::Public)
                | Tok::Kw(Kw::Class)
                | Tok::Kw(Kw::Interface)
                | Tok::Kw(Kw::Enum)
                | Tok::Punct("@") => {
                    if let Some(item) = self.parse_type_decl(docs) {
                        match item {
                            TopItem::Class(c) => prog.classes.push(c),
                            TopItem::Interface(i) => prog.interfaces.push(i),
                            TopItem::Enum(e) => prog.enums.push(e),
                        }
                    }
                }
                _ => {
                    self.err("顶层只允许 `public class` / `public interface` / `public enum` / `import`");
                    self.bump();
                }
            }
            if self.pos == start && !self.at_eof() {
                self.bump();
            }
        }
        prog
    }

    fn parse_type_decl(&mut self, docs: Vec<String>) -> Option<TopItem> {
        let attrs = self.parse_annotations();
        self.eat_kw(Kw::Public);
        if self.check_kw(Kw::Class) {
            return Some(TopItem::Class(self.parse_class(attrs, docs)));
        }
        if self.check_kw(Kw::Interface) {
            return Some(TopItem::Interface(self.parse_interface(attrs, docs)));
        }
        if self.check_kw(Kw::Enum) {
            return Some(TopItem::Enum(self.parse_enum(attrs, docs)));
        }
        self.err("这里期望 `class`、`interface` 或 `enum`");
        self.bump();
        None
    }

    /// 带数据枚举：
    /// ```
    /// enum Shape { Circle(double r), Rect(double w, double h), Empty }
    /// ```
    fn parse_enum(&mut self, attrs: Vec<Attr>, docs: Vec<String>) -> EnumDecl {
        let line = self.line();
        self.bump(); // enum
        let name = self.expect_ident();
        let type_params = if self.check_punct("<") {
            self.parse_type_params()
        } else {
            Vec::new()
        };
        let mut variants = Vec::new();
        if self.expect_punct("{") {
            while !self.at_eof() && !self.check_punct("}") {
                let start = self.pos;
                let vdocs = self.parse_docs();
                let vline = self.line();
                let vname = self.expect_ident();
                let params = if self.check_punct("(") {
                    self.parse_params()
                } else {
                    Vec::new()
                };
                variants.push(EnumVariant {
                    name: vname,
                    params,
                    docs: vdocs,
                    line: vline,
                });
                if !self.eat_punct(",") {
                    break;
                }
                if self.pos == start && !self.at_eof() {
                    self.bump();
                }
            }
            self.expect_punct("}");
        }
        EnumDecl {
            name,
            attrs,
            docs,
            type_params,
            variants,
            line,
        }
    }

    fn parse_class(&mut self, attrs: Vec<Attr>, docs: Vec<String>) -> ClassDecl {
        let line = self.line();
        self.bump(); // class
        let name = self.expect_ident();

        let prev_name = std::mem::replace(&mut self.class_name, name.clone());

        let type_params = if self.check_punct("<") {
            self.parse_type_params()
        } else {
            Vec::new()
        };

        let mut implements = Vec::new();
        let mut is_exception = false;
        if self.eat_kw(Kw::Extends) {
            // JR-C6：只允许 `extends Exception` / `extends RuntimeException`（自定义异常），
            // 其余父类仍按「Rust 没有继承」报错。
            let mut segs = vec![self.expect_ident()];
            while self.eat_punct(".") {
                segs.push(self.expect_ident());
            }
            let base = segs.last().cloned().unwrap_or_default();
            if base == "Exception" || base == "RuntimeException" {
                is_exception = true;
            } else {
                self.diags.push(
                    Diag::error(line, "jrust 不支持类继承（extends）").with_hint(
                        "Rust 没有继承：共享行为抽成 interface（→ trait），共享数据放进字段，组合优于继承。唯一例外是 `extends Exception`（自定义异常）。",
                    ),
                );
            }
            self.skip_generic_args();
        }
        if self.eat_kw(Kw::Implements) {
            loop {
                let iface = self.expect_ident();
                let ity = if self.check_punct("<") {
                    let aline = self.line();
                    let args = self.parse_generic_args();
                    if args.is_empty() {
                        self.diags.push(
                            Diag::error(aline, "接口的泛型实参不能为空").with_hint(
                                "写全类型实参，如 `implements Mapper<Integer>`。",
                            ),
                        );
                    }
                    Type::Generic(iface, args)
                } else {
                    Type::Named(iface)
                };
                implements.push(ity);
                if !self.eat_punct(",") {
                    break;
                }
            }
        }

        let mut class = ClassDecl {
            name: name.clone(),
            attrs,
            docs,
            is_exception,
            type_params,
            implements,
            fields: Vec::new(),
            ctor: None,
            methods: Vec::new(),
            raw_members: Vec::new(),
            line,
        };

        if self.expect_punct("{") {
            while !self.at_eof() && !self.check_punct("}") {
                let start = self.pos;
                let mdocs = self.parse_docs();
                self.parse_member(&mut class, mdocs);
                if self.pos == start && !self.at_eof() {
                    self.bump();
                }
            }
            self.expect_punct("}");
        }

        self.class_name = prev_name;
        class
    }

    /// `raw` 块不得嵌套（JR-X3）。
    fn reject_nested_raw(&mut self, code: &str, line: usize) {
        if contains_nested_raw(code) {
            self.diags.push(
                Diag::error_code(line, "JR3027", "raw 块不能嵌套")
                    .with_hint("把内层 Rust 代码直接并入外层 raw，或拆成相邻的两个 raw 块。"),
            );
        }
    }

    fn parse_member(&mut self, class: &mut ClassDecl, docs: Vec<String>) {
        // 类内 `raw { ... }`（JR-X1）：作为成员原样收集。
        if let Tok::Raw(code) = self.peek().clone() {
            let line = self.line();
            self.reject_nested_raw(&code, line);
            self.bump();
            class.raw_members.push(RawCode { code, line });
            return;
        }
        if self.check_kw(Kw::Raw) {
            self.err_hint(
                "raw 必须写成 `raw { ... }`",
                "在 `raw` 后紧跟花括号，里面直接写 Rust 代码。",
            );
            self.bump();
            return;
        }
        let attrs = self.parse_annotations();
        if self.at_eof() || self.check_punct("}") {
            return;
        }

        let mut vis = Vis::Package;
        let mut is_static = false;
        let mut is_final = false;
        let mut is_async = false;
        let mut is_synchronized = false;
        loop {
            if self.eat_kw(Kw::Public) {
                vis = Vis::Public;
            } else if self.eat_kw(Kw::Private) {
                vis = Vis::Private;
            } else if self.check_kw(Kw::Protected) {
                self.err_hint(
                    "jrust 不支持 protected 可见性",
                    "Rust 只有 pub / 私有两档：子类可见的语义没有对应物（因为也没有继承）。",
                );
                self.bump();
            } else if self.eat_kw(Kw::Static) {
                is_static = true;
            } else if self.eat_kw(Kw::Async) {
                is_async = true;
            } else if self.eat_kw(Kw::Synchronized) {
                is_synchronized = true;
            } else if self.eat_kw(Kw::Final) {
                // final 对字段是「不可变」（static final → const），对方法是「不可覆写」
                is_final = true;
            } else {
                break;
            }
        }

        // 方法级类型参数：`public <T> T id(T x)`。
        let type_params = if self.check_punct("<") {
            self.parse_type_params()
        } else {
            Vec::new()
        };

        // 构造器：名字 == 类名，且后面紧跟 `(`
        if let Tok::Ident(n) = self.peek().clone() {
            if n == class.name && matches!(self.peek_at(1), Tok::Punct("(")) {
                if !type_params.is_empty() {
                    self.err_hint(
                        "jrust MVP 不支持构造器单独声明类型参数",
                        "把类型参数提到类上（`class Box<T>`），或改用具体类型。",
                    );
                }
                let ctor_line = self.line();
                self.bump(); // 类名
                let params = self.parse_params();
                let body = self.parse_block();
                if class.ctor.is_some() {
                    self.diags.push(Diag::error(
                        ctor_line,
                        "jrust MVP 每个类只允许一个构造器",
                    ));
                }
                class.ctor = Some(Constructor {
                    vis,
                    attrs,
                    docs,
                    params,
                    body,
                    line: ctor_line,
                });
                return;
            }
        }

        let member_line = self.line();
        let ty = match self.parse_type() {
            Some(t) => t,
            None => {
                self.skip_to_stmt_end();
                return;
            }
        };
        let name = self.expect_ident();

        if self.check_punct("(") {
            let mline = self.line();
            let params = self.parse_method_params();
            let mut throws = Vec::new();
            if self.eat_kw(Kw::Throws) {
                loop {
                    let mut n = self.expect_ident();
                    while self.eat_punct(".") {
                        let seg = self.expect_ident();
                        n.push('.');
                        n.push_str(&seg);
                    }
                    throws.push(n);
                    if !self.eat_punct(",") {
                        break;
                    }
                }
            }
            let body = self.parse_block();
            class.methods.push(Method {
                vis,
                is_static,
                attrs,
                is_async,
                is_synchronized,
                docs,
                type_params,
                name,
                ret: ty,
                params,
                throws,
                body,
                line: mline,
            });
        } else {
            if !type_params.is_empty() {
                self.err_hint(
                    "字段不能声明类型参数",
                    "类型参数写在类上（`class Box<T>`）；字段类型直接用 `T`。",
                );
            }
            if is_static {
                if !is_final {
                    self.diags.push(
                        Diag::error_code(
                            member_line,
                            "JR3013",
                            "jrust 不支持可变的静态字段（static 非 final）",
                        )
                        .with_hint(
                            "Rust 的可变全局需要 `static mut` + unsafe，与值语义冲突；把状态放进对象字段，或用参数传递。",
                        ),
                    );
                    if self.eat_punct("=") {
                        let _ = self.parse_expr();
                    }
                    self.eat_punct(";");
                } else if self.eat_punct("=") {
                    let init = self.parse_expr();
                    self.expect_punct(";");
                    class.fields.push(Field {
                        vis,
                        is_static: true,
                        is_final: true,
                        init: Some(init),
                        attrs,
                        docs,
                        ty,
                        name,
                        line: member_line,
                    });
                } else {
                    self.err_hint(
                        "静态常量必须带初始值",
                        "写成 `public static final int N = 3;`；Rust 的 `const` 要求声明处给出编译期常量。",
                    );
                    self.eat_punct(";");
                }
            } else {
                if self.check_punct("=") {
                    self.err_hint(
                        "jrust MVP 不支持字段初始化",
                        "在构造器里给字段赋值；Rust 对应结构体字面量 `Self { field: value }`。",
                    );
                    self.skip_to_stmt_end();
                } else {
                    self.expect_punct(";");
                }
                class.fields.push(Field {
                    vis,
                    is_static: false,
                    is_final,
                    init: None,
                    attrs,
                    docs,
                    ty,
                    name,
                    line: member_line,
                });
            }
        }
    }

    fn parse_interface(&mut self, attrs: Vec<Attr>, docs: Vec<String>) -> InterfaceDecl {
        let line = self.line();
        self.bump(); // interface
        let name = self.expect_ident();
        let type_params = if self.check_punct("<") {
            self.parse_type_params()
        } else {
            Vec::new()
        };
        let mut extends = Vec::new();
        if self.eat_kw(Kw::Extends) {
            loop {
                match self.parse_type() {
                    Some(t) => extends.push(t),
                    None => break,
                }
                if !self.eat_punct(",") {
                    break;
                }
            }
        }
        let mut methods = Vec::new();
        if self.expect_punct("{") {
            while !self.at_eof() && !self.check_punct("}") {
                let start = self.pos;
                let mdocs = self.parse_docs();
                let mline = self.line();
                let mut saw_static = false;
                loop {
                    if self.eat_kw(Kw::Public) || self.eat_kw(Kw::Final) || self.eat_kw(Kw::Default)
                    {
                        continue;
                    }
                    if self.check_kw(Kw::Static) {
                        self.bump();
                        saw_static = true;
                        continue;
                    }
                    break;
                }
                let mattrs = self.parse_annotations();
                let mtype_params = if self.check_punct("<") {
                    self.parse_type_params()
                } else {
                    Vec::new()
                };
                match self.parse_type() {
                    Some(ret) => {
                        let mname = self.expect_ident();
                        let params = self.parse_method_params();
                        // `;` = 只有签名；`{ ... }` = 默认方法 / static 方法体（JR-D10）。
                        let body = if self.eat_punct(";") {
                            None
                        } else {
                            Some(self.parse_block())
                        };
                        methods.push(MethodSig {
                            attrs: mattrs,
                            docs: mdocs,
                            type_params: mtype_params,
                            name: mname,
                            ret,
                            params,
                            is_static: saw_static,
                            body,
                            line: mline,
                        });
                    }
                    None => self.skip_to_stmt_end(),
                }
                if self.pos == start && !self.at_eof() {
                    self.bump();
                }
            }
            self.expect_punct("}");
        }
        InterfaceDecl {
            name,
            attrs,
            docs,
            extends,
            type_params,
            methods,
            line,
        }
    }

    // ---------- 类型 ----------

    /// 解析声明侧的类型参数列表：`<T, U>` 或带界 `<T extends A & B>`。
    fn parse_type_params(&mut self) -> Vec<TypeParam> {
        let mut v = Vec::new();
        if !self.eat_punct("<") {
            return v;
        }
        if self.eat_punct(">") {
            self.err("类型参数列表不能为空");
            return v;
        }
        loop {
            let name = self.expect_ident();
            let mut bounds = Vec::new();
            if self.eat_kw(Kw::Extends) {
                loop {
                    match self.parse_type() {
                        Some(t) => bounds.push(t),
                        None => break,
                    }
                    if !self.eat_punct("&") {
                        break;
                    }
                }
            }
            v.push(TypeParam { name, bounds });
            if self.eat_punct(",") {
                continue;
            }
            break;
        }
        self.expect_punct(">");
        v
    }

    /// 解析使用侧的类型实参：`<Integer>` 或钻石 `<>`（返回空列表）。
    fn parse_generic_args(&mut self) -> Vec<Type> {
        let mut v = Vec::new();
        if !self.eat_punct("<") {
            return v;
        }
        if self.eat_punct(">") {
            return v;
        }
        loop {
            match self.parse_type() {
                Some(t) => v.push(t),
                None => break,
            }
            if self.eat_punct(",") {
                continue;
            }
            break;
        }
        self.expect_punct(">");
        v
    }

    fn parse_type(&mut self) -> Option<Type> {
        let line = self.line();
        // Java 的声明处变型：`?`、`? extends T`、`? super T`，Rust 没有对应机制。
        if self.check_punct("?") {
            self.bump();
            let mut has_bound = false;
            if self.eat_kw(Kw::Extends) {
                has_bound = true;
            } else if matches!(self.peek(), Tok::Ident(n) if n == "super") {
                self.bump();
                has_bound = true;
            }
            self.diags.push(
                Diag::error_code(
                    line,
                    "JR3010",
                    "jrust 不支持泛型通配符（`? extends` / `? super`）",
                )
                .with_hint(
                    "Rust 没有声明处变型：改用 trait object（`Box<dyn Trait>`）或写出具体类型。",
                ),
            );
            if has_bound {
                let _ = self.parse_type();
            }
            return None;
        }
        let mut base = match self.peek().clone() {
            Tok::Kw(Kw::Int) => {
                self.bump();
                Type::Int
            }
            Tok::Kw(Kw::Long) => {
                self.bump();
                Type::Long
            }
            Tok::Kw(Kw::Double) => {
                self.bump();
                Type::Double
            }
            Tok::Kw(Kw::Float) => {
                self.bump();
                Type::Float
            }
            Tok::Kw(Kw::Boolean) => {
                self.bump();
                Type::Boolean
            }
            Tok::Kw(Kw::Char) => {
                self.bump();
                Type::Char
            }
            Tok::Kw(Kw::Void) => {
                self.bump();
                Type::Void
            }
            Tok::Ident(name) => {
                self.bump();
                match name.as_str() {
                    "String" => Type::String,
                    // Java 包装类型按对应的 Rust 标量处理（Map 的键值类型常用）。
                    "Integer" => Type::Int,
                    "Long" => Type::Long,
                    "Double" => Type::Double,
                    "Float" => Type::Float,
                    "Boolean" => Type::Boolean,
                    "Character" => Type::Char,
                    // Rust 原生标量类型名，直接接受并原样输出。
                    "i8" | "i16" | "i32" | "i64" | "i128" | "isize" | "u8" | "u16" | "u32"
                    | "u64" | "u128" | "usize" | "f32" | "f64" | "bool" => {
                        Type::RustPrim(name)
                    }
                    "List" | "ArrayList" | "Collection" | "LinkedList" | "Vec" => {
                        if self.eat_punct("<") {
                            let inner = self.parse_type().unwrap_or(Type::String);
                            self.expect_punct(">");
                            Type::List(Box::new(inner))
                        } else {
                            self.diags.push(
                                Diag::error(line, "jrust 不支持 raw type（不带类型参数）")
                                    .with_hint(
                                        "写成 `List<Animal>` 或 `Vec<Animal>`；Rust 的 Vec<T> 必须写明元素类型，没有泛型擦除。",
                                    ),
                            );
                            Type::List(Box::new(Type::String))
                        }
                    }
                    "Map" | "HashMap" => {
                        if self.eat_punct("<") {
                            let k = self.parse_type().unwrap_or(Type::String);
                            self.expect_punct(",");
                            let v = self.parse_type().unwrap_or(Type::String);
                            self.expect_punct(">");
                            Type::Map(Box::new(k), Box::new(v))
                        } else {
                            self.diags.push(
                                Diag::error(line, "jrust 不支持 raw type（不带类型参数的 Map）")
                                    .with_hint(
                                        "写成 `Map<String, Integer>`；Rust 的 HashMap<K, V> 必须写明键值类型，没有泛型擦除。",
                                    ),
                            );
                            Type::Map(Box::new(Type::String), Box::new(Type::String))
                        }
                    }
                    "Set" | "HashSet" => {
                        if self.eat_punct("<") {
                            let inner = self.parse_type().unwrap_or(Type::String);
                            self.expect_punct(">");
                            Type::Set(Box::new(inner))
                        } else {
                            self.diags.push(
                                Diag::error(line, "jrust 不支持 raw type（不带类型参数的 Set）")
                                    .with_hint(
                                        "写成 `Set<Integer>`；Rust 的 HashSet<T> 必须写明元素类型，没有泛型擦除。",
                                    ),
                            );
                            Type::Set(Box::new(Type::String))
                        }
                    }
                    _ => Type::Named(name),
                }
            }
            _ => {
                let d = self.describe();
                self.err(format!("这里期望一个类型，实际是 {}", d));
                return None;
            }
        };

        // 使用侧类型实参：`Box<Integer>`、`Pair<String, Integer>`。
        if let Type::Named(n) = &base {
            if self.check_punct("<") {
                let n = n.clone();
                let aline = self.line();
                let args = self.parse_generic_args();
                if args.is_empty() {
                    self.diags.push(
                        Diag::error(aline, "类型位置的泛型实参不能为空").with_hint(
                            "钻石写法 `<>` 只能跟在 `new` 后面；声明变量时写全，如 `Box<Integer> b`。",
                        ),
                    );
                }
                base = Type::Generic(n, args);
            }
        }

        while self.check_punct("[") && matches!(self.peek_at(1), Tok::Punct("]")) {
            self.bump();
            self.bump();
            base = Type::Array(Box::new(base));
        }
        // `T?` → `Option<T>`。类型位置上的 `?` 与表达式里的错误传播 `?` 不冲突：
        // parse_type 只在期望类型的地方调用。
        while self.eat_punct("?") {
            base = Type::Nullable(Box::new(base));
        }
        Some(base)
    }

    fn parse_params(&mut self) -> Vec<Param> {
        self.parse_params_impl(false)
    }

    /// 方法 / 接口方法形参：允许 `@ref` / `@mut`（Stage D）。
    fn parse_method_params(&mut self) -> Vec<Param> {
        self.parse_params_impl(true)
    }

    fn parse_params_impl(&mut self, allow_borrow: bool) -> Vec<Param> {
        let mut v = Vec::new();
        if !self.expect_punct("(") {
            return v;
        }
        if self.eat_punct(")") {
            return v;
        }
        loop {
            let pline = self.line();
            let attrs = if allow_borrow {
                self.parse_param_annotations()
            } else {
                self.parse_annotations()
            };
            let ty = match self.parse_type() {
                Some(t) => t,
                None => Type::Int,
            };
            let name = self.expect_ident();
            v.push(Param {
                ty,
                name,
                attrs,
                line: pline,
            });
            if self.eat_punct(",") {
                continue;
            }
            break;
        }
        self.expect_punct(")");
        v
    }

    // ---------- 语句 ----------

    fn parse_block(&mut self) -> Vec<Stmt> {
        let mut v = Vec::new();
        if !self.expect_punct("{") {
            return v;
        }
        while !self.at_eof() && !self.check_punct("}") {
            let start = self.pos;
            // 方法体内的 `///` 没有对应 Rust item，按普通注释丢弃。
            let _ = self.parse_docs();
            if let Some(s) = self.parse_stmt() {
                v.push(s);
            }
            if self.pos == start && !self.at_eof() {
                self.bump();
            }
        }
        self.expect_punct("}");
        v
    }

    fn parse_body(&mut self) -> Vec<Stmt> {
        if self.check_punct("{") {
            self.parse_block()
        } else {
            self.parse_stmt().map(|s| vec![s]).unwrap_or_default()
        }
    }

    fn parse_paren_expr(&mut self) -> Expr {
        self.expect_punct("(");
        let e = self.parse_expr();
        self.expect_punct(")");
        e
    }

    /// `break outer;` / `continue outer;`：消费标签并报 JR2004（JR-E6）。
    fn reject_labeled_jump(&mut self, kw: &str) {
        if let Tok::Ident(label) = self.peek().clone() {
            let line = self.line();
            self.bump();
            self.diags.push(
                Diag::error_code(
                    line,
                    "JR2004",
                    format!("jrust 不支持带标签的 `{} {}`", kw, label),
                )
                .with_hint(
                    "Rust 有 `break 'label;`，但 jrust 不暴露标签：用布尔标志或把循环抽成独立方法。",
                ),
            );
        }
    }

    fn parse_stmt(&mut self) -> Option<Stmt> {
        let line = self.line();
        // `outer: for (...) { ... }`：jrust 不支持标签（JR-E6）。
        if matches!(self.peek(), Tok::Ident(_)) && matches!(self.peek_at(1), Tok::Punct(p) if *p == ":") {
            let label = self.expect_ident();
            self.bump(); // :
            self.diags.push(
                Diag::error_code(line, "JR2004", format!("jrust 不支持带标签的循环 `{}:`", label))
                    .with_hint(
                        "Rust 有 `'label: loop { break 'label; }`，但 jrust 不暴露标签：用布尔标志或把循环抽成独立方法。",
                    ),
            );
            return self.parse_stmt();
        }
        match self.peek().clone() {
            Tok::Raw(code) => {
                let line = self.line();
                self.reject_nested_raw(&code, line);
                self.bump();
                Some(Stmt::Raw { code, line })
            }
            Tok::Kw(Kw::Raw) => {
                self.err_hint(
                    "raw 必须写成 `raw { ... }`",
                    "在 `raw` 后紧跟花括号，里面直接写 Rust 代码。",
                );
                self.bump();
                None
            }
            Tok::Punct("{") => {
                self.err_hint(
                    "jrust 不支持裸代码块",
                    "把语句直接写在方法里，或放进 if / while 的块中。",
                );
                self.skip_block();
                None
            }
            Tok::Kw(Kw::If) => {
                self.bump();
                let cond = self.parse_paren_expr();
                let then = self.parse_body();
                let els = if self.eat_kw(Kw::Else) {
                    if self.check_kw(Kw::If) {
                        self.parse_stmt().map(|s| vec![s])
                    } else {
                        Some(self.parse_body())
                    }
                } else {
                    None
                };
                Some(Stmt::If {
                    cond,
                    then,
                    els,
                    line,
                })
            }
            Tok::Kw(Kw::While) => {
                self.bump();
                let cond = self.parse_paren_expr();
                let body = self.parse_body();
                Some(Stmt::While { cond, body, line })
            }
            Tok::Kw(Kw::For) => {
                self.bump();
                if self.check_punct("(") && self.is_foreach_paren() {
                    self.bump(); // (
                    let ty = match self.parse_type() {
                        Some(t) => t,
                        None => Type::Int,
                    };
                    let name = self.expect_ident();
                    self.expect_punct(":");
                    let iter = self.parse_expr();
                    self.expect_punct(")");
                    let body = self.parse_body();
                    Some(Stmt::ForEach {
                        ty,
                        name,
                        iter,
                        body,
                        line,
                    })
                } else if self.check_punct("(") {
                    self.parse_c_for(line)
                } else {
                    self.err_hint(
                        "`for` 后面缺少 `(`",
                        "写成 `for (int i = 0; i < n; i++) { ... }` 或 `for (T x : xs) { ... }`。",
                    );
                    self.skip_body();
                    None
                }
            }
            Tok::Kw(Kw::Return) => {
                self.bump();
                if self.eat_punct(";") || self.check_punct("}") {
                    Some(Stmt::Return { value: None, line })
                } else {
                    let value = self.parse_expr();
                    if !self.eat_punct(";") && !self.check_punct("}") {
                        self.err("`return` 语句末尾缺少 `;`");
                    }
                    Some(Stmt::Return {
                        value: Some(value),
                        line,
                    })
                }
            }
            Tok::Kw(Kw::Break) => {
                self.bump();
                self.reject_labeled_jump("break");
                self.expect_punct(";");
                Some(Stmt::Break { line })
            }
            Tok::Kw(Kw::Continue) => {
                self.bump();
                self.reject_labeled_jump("continue");
                self.expect_punct(";");
                Some(Stmt::Continue { line })
            }
            Tok::Kw(Kw::Switch) => self.parse_switch(line),
            Tok::Kw(Kw::Try) | Tok::Kw(Kw::Catch) | Tok::Kw(Kw::Finally) => {
                self.diags.push(
                    Diag::error(line, "jrust 不支持 try / catch / finally").with_hint(
                        "Rust 没有异常：可能失败的操作返回 Result<T, E>，调用处用 `?` 向上传播；清理资源交给 Drop（析构）。",
                    ),
                );
                self.bump();
                if self.check_punct("{") {
                    self.skip_block();
                }
                if self.check_kw(Kw::Catch) {
                    self.bump();
                    self.skip_parens();
                    self.skip_block();
                }
                if self.check_kw(Kw::Finally) {
                    self.bump();
                    self.skip_block();
                }
                None
            }
            Tok::Kw(Kw::Spawn) => {
                self.bump();
                let body = if self.check_punct("{") {
                    self.parse_block()
                } else {
                    let expr = self.parse_expr();
                    self.expect_punct(";");
                    vec![Stmt::Expr { expr, line }]
                };
                Some(Stmt::Spawn { body, line })
            }
            Tok::Kw(Kw::Throw) => {
                self.bump();
                let expr = self.parse_expr();
                self.expect_punct(";");
                Some(Stmt::Throw { expr, line })
            }
            Tok::Kw(Kw::Synchronized) => {
                self.bump();
                if !self.eat_punct("(") {
                    self.err_hint(
                        "`synchronized` 后面必须跟 `(锁对象)`",
                        "写成 `synchronized (this.f) { ... }`，`f` 应是 `@shared` 可变字段。",
                    );
                    self.skip_to_stmt_end();
                    return None;
                }
                let lock = self.parse_expr();
                self.expect_punct(")");
                let body = self.parse_block();
                Some(Stmt::Sync { lock, body, line })
            }
            _ => {
                if self.looks_like_decl() {
                    self.parse_let_stmt(line)
                } else {
                    self.parse_expr_stmt(line)
                }
            }
        }
    }

    /// `switch (x) { case ...: ... default: ... }` → 交给 codegen 生成 `match`。
    fn parse_switch(&mut self, line: usize) -> Option<Stmt> {
        self.bump(); // switch
        self.expect_punct("(");
        let subject = self.parse_expr();
        self.expect_punct(")");
        let mut cases = Vec::new();
        let mut default: Option<Vec<Stmt>> = None;
        if !self.expect_punct("{") {
            return None;
        }
        while !self.at_eof() && !self.check_punct("}") {
            let start = self.pos;
            if self.eat_kw(Kw::Case) {
                let cline = self.line();
                let pattern = self.parse_case_pattern();
                let guard = if self.eat_kw(Kw::If) {
                    Some(self.parse_expr())
                } else {
                    None
                };
                self.expect_punct(":");
                let body = self.parse_case_body();
                cases.push(SwitchCase {
                    pattern,
                    guard,
                    body,
                    line: cline,
                });
            } else if self.eat_kw(Kw::Default) {
                self.expect_punct(":");
                if default.is_some() {
                    self.err("switch 只能有一个 default 分支");
                }
                default = Some(self.parse_case_body());
            } else {
                self.err("switch 里只允许 `case` 或 `default`");
                self.bump();
            }
            if self.pos == start && !self.at_eof() {
                self.bump();
            }
        }
        self.expect_punct("}");
        Some(Stmt::Switch {
            subject,
            cases,
            default,
            line,
        })
    }

    fn parse_case_pattern(&mut self) -> CasePattern {
        match self.peek().clone() {
            Tok::Kw(Kw::True) => {
                self.bump();
                CasePattern::Bool(true)
            }
            Tok::Kw(Kw::False) => {
                self.bump();
                CasePattern::Bool(false)
            }
            Tok::Ident(first) => {
                self.bump();
                let (enum_name, name) = if self.eat_punct(".") {
                    (Some(first), self.expect_ident())
                } else {
                    (None, first)
                };
                let bindings = if self.check_punct("(") {
                    self.parse_case_bindings()
                } else {
                    Vec::new()
                };
                CasePattern::Variant {
                    enum_name,
                    name,
                    bindings,
                }
            }
            _ => {
                // 字面量分支（int / String……）：codegen 会以 JR3019 拒绝，因为 Rust 的 match 需要穷尽。
                let e = self.parse_expr();
                CasePattern::Other(e)
            }
        }
    }

    /// `case Circle(double r)` 里的绑定：类型可省略。
    fn parse_case_bindings(&mut self) -> Vec<String> {
        let mut v = Vec::new();
        if !self.expect_punct("(") {
            return v;
        }
        if self.eat_punct(")") {
            return v;
        }
        loop {
            if self.looks_like_typed_binding() {
                let _ = self.parse_type();
            }
            v.push(self.expect_ident());
            if self.eat_punct(",") {
                continue;
            }
            break;
        }
        self.expect_punct(")");
        v
    }

    /// 当前是否形如 `Type name`（类型 + 标识符），用于区分 `Circle(double r)` 和 `Circle(r)`。
    fn looks_like_typed_binding(&self) -> bool {
        match self.peek() {
            Tok::Kw(Kw::Int)
            | Tok::Kw(Kw::Long)
            | Tok::Kw(Kw::Double)
            | Tok::Kw(Kw::Float)
            | Tok::Kw(Kw::Boolean)
            | Tok::Kw(Kw::Char) => true,
            Tok::Ident(_) => {
                matches!(self.peek_at(1), Tok::Ident(_) | Tok::Punct("<"))
                    // `Foo[] x`
                    || (matches!(self.peek_at(1), Tok::Punct("["))
                        && matches!(self.peek_at(2), Tok::Punct("]")))
                    // `Foo? x`
                    || (matches!(self.peek_at(1), Tok::Punct("?"))
                        && matches!(self.peek_at(2), Tok::Ident(_)))
            }
            _ => false,
        }
    }

    fn parse_case_body(&mut self) -> Vec<Stmt> {
        let mut v = Vec::new();
        while !self.at_eof()
            && !self.check_punct("}")
            && !self.check_kw(Kw::Case)
            && !self.check_kw(Kw::Default)
        {
            let start = self.pos;
            if let Some(s) = self.parse_stmt() {
                v.push(s);
            }
            if self.pos == start && !self.at_eof() {
                self.bump();
            }
        }
        v
    }

    fn is_foreach_paren(&self) -> bool {
        let mut depth = 0i32;
        let mut i = self.pos;
        while i < self.toks.len() {
            match &self.toks[i].tok {
                Tok::Punct("(") => depth += 1,
                Tok::Punct(")") => {
                    depth -= 1;
                    if depth <= 0 {
                        return false;
                    }
                }
                Tok::Punct(";") if depth == 1 => return false,
                Tok::Punct(":") => return true,
                Tok::Eof => return false,
                _ => {}
            }
            i += 1;
        }
        false
    }

    /// C 风格计数循环。形状不支持时统一报 JR2003 并跳到循环体之后，
    /// 不生成可能死循环或语义不同的代码。
    fn parse_c_for(&mut self, line: usize) -> Option<Stmt> {
        self.expect_punct("(");
        let ty = if self.eat_kw(Kw::Int) {
            Type::Int
        } else if self.eat_kw(Kw::Long) {
            Type::Long
        } else {
            return self.c_for_bail(
                line,
                "C 风格 for 的初始化只支持 `int i = ...` / `long i = ...`",
                "写成 `for (int i = 0; i < n; i++) { ... }`。",
            );
        };
        let name = self.expect_ident();
        if !self.eat_punct("=") {
            return self.c_for_bail(
                line,
                "C 风格 for 的初始化缺少 `=`",
                "写成 `for (int i = 0; i < n; i++) { ... }`。",
            );
        }
        let start = self.parse_expr();
        if !self.eat_punct(";") {
            return self.c_for_bail(
                line,
                "C 风格 for 的初始化后缺少 `;`",
                "三段之间用 `;` 分隔：`for (int i = 0; i < n; i++)`。",
            );
        }
        let cond = self.parse_expr();
        if !self.eat_punct(";") {
            return self.c_for_bail(
                line,
                "C 风格 for 的条件后缺少 `;`",
                "三段之间用 `;` 分隔：`for (int i = 0; i < n; i++)`。",
            );
        }
        let step_name = self.expect_ident();
        let (descending, step_ok) = if self.eat_punct("++") {
            (false, true)
        } else if self.eat_punct("--") {
            (true, true)
        } else if self.eat_punct("+=") {
            (false, self.eat_int_one())
        } else if self.eat_punct("-=") {
            (true, self.eat_int_one())
        } else {
            (false, false)
        };
        if !step_ok {
            return self.c_for_bail(
                line,
                "C 风格 for 的步进只支持 `i++`、`i--`、`i += 1`、`i -= 1`",
                "步长不为 1 时改用 while：`int i = 0; while (i < n) { ...; i += 2; }`。",
            );
        }
        if step_name != name {
            return self.c_for_bail(
                line,
                format!(
                    "C 风格 for 的步进变量 `{}` 与初始化变量 `{}` 不一致",
                    step_name, name
                ),
                "三段都用同一个计数变量，例如 `for (int i = 0; i < n; i++)`。",
            );
        }
        let (op, lhs, rhs) = match cond {
            Expr::Binary { op, lhs, rhs } => (op, lhs, rhs),
            _ => {
                return self.c_for_bail(
                    line,
                    "C 风格 for 的条件必须是 `i < n` 形式的比较",
                    "条件写成 `i < n`、`i <= n`、`i > n` 或 `i >= n`。",
                )
            }
        };
        let (inclusive, cond_descending) = match op {
            BinOp::Lt => (false, false),
            BinOp::Le => (true, false),
            BinOp::Gt => (false, true),
            BinOp::Ge => (true, true),
            _ => {
                return self.c_for_bail(
                    line,
                    "C 风格 for 的条件只支持 `<`、`<=`、`>`、`>=`",
                    "条件写成 `i < n`、`i <= n`、`i > n` 或 `i >= n`。",
                )
            }
        };
        if !matches!(&*lhs, Expr::Ident(n) if *n == name) {
            return self.c_for_bail(
                line,
                "C 风格 for 的条件左侧必须是计数变量",
                format!("写成 `{} < n` 这样的形式。", name),
            );
        }
        if cond_descending != descending {
            return self.c_for_bail(
                line,
                "C 风格 for 的步进方向与条件方向不一致",
                "自增配 `i < n` / `i <= n`，自减配 `i > n` / `i >= n`。",
            );
        }
        if !self.eat_punct(")") {
            return self.c_for_bail(
                line,
                "C 风格 for 头部缺少 `)`",
                "三段之后用 `)` 收尾：`for (int i = 0; i < n; i++)`。",
            );
        }
        let body = self.parse_body();
        Some(Stmt::ForRange {
            ty,
            name,
            start,
            end: *rhs,
            inclusive,
            descending,
            body,
            line,
        })
    }

    fn c_for_bail(
        &mut self,
        line: usize,
        msg: impl Into<String>,
        hint: impl Into<String>,
    ) -> Option<Stmt> {
        self.diags
            .push(Diag::error_code(line, "JR2003", msg).with_hint(hint));
        self.skip_until_rparen();
        self.skip_body();
        None
    }

    /// `(` 已被消费时，跳到与之配对的 `)` 之后。
    fn skip_until_rparen(&mut self) {
        let mut depth = 1i32;
        while !self.at_eof() {
            if self.check_punct("(") {
                depth += 1;
                self.bump();
            } else if self.check_punct(")") {
                depth -= 1;
                self.bump();
                if depth == 0 {
                    break;
                }
            } else {
                self.bump();
            }
        }
    }

    /// 只接受步长 1：`i += 1` / `i -= 1`。
    fn eat_int_one(&mut self) -> bool {
        if matches!(self.peek(), Tok::Int(1)) {
            self.bump();
            true
        } else {
            false
        }
    }

    /// `new T[n]`：只在「类型 token 紧跟 `[`」时接管。
    ///
    /// 元素类型限定为标量 / String（都能给出确定默认值）；`new Foo[n]` 这类需要
    /// `Default` 的类型报 `JR3014`，并提示改用 `Vec` + `push`。
    fn try_new_array(&mut self) -> Option<Expr> {
        if !matches!(self.peek_at(1), Tok::Punct("[")) {
            return None;
        }
        let elem = match self.peek() {
            Tok::Kw(Kw::Int) => Some(Type::Int),
            Tok::Kw(Kw::Long) => Some(Type::Long),
            Tok::Kw(Kw::Double) => Some(Type::Double),
            Tok::Kw(Kw::Float) => Some(Type::Float),
            Tok::Kw(Kw::Boolean) => Some(Type::Boolean),
            Tok::Kw(Kw::Char) => Some(Type::Char),
            Tok::Ident(n) => match n.as_str() {
                "String" => Some(Type::String),
                "Integer" => Some(Type::Int),
                "Long" => Some(Type::Long),
                "Double" => Some(Type::Double),
                "Float" => Some(Type::Float),
                "Boolean" => Some(Type::Boolean),
                "Character" => Some(Type::Char),
                "i8" | "i16" | "i32" | "i64" | "i128" | "isize" | "u8" | "u16" | "u32"
                | "u64" | "u128" | "usize" | "f32" | "f64" | "bool" => {
                    Some(Type::RustPrim(n.clone()))
                }
                _ => None,
            },
            _ => None,
        };
        self.bump(); // 元素类型
        self.bump(); // [
        if self.check_punct("]") {
            self.bump();
            self.err_hint(
                "数组字面量初始化 `new T[]{...}` 暂不支持",
                "长度写在方括号里：`new int[3]`；要初值就逐个下标赋值，或改用 `List` + `add`。",
            );
            self.skip_body();
            return Some(Expr::Ident("<错误>".to_string()));
        }
        let len = self.parse_expr();
        self.expect_punct("]");
        if self.check_punct("[") {
            self.err_hint(
                "不支持多维数组 `new T[n][m]`",
                "用 `List<List<T>>` 逐层创建，或先建一维 `new T[n * m]` 再自己算下标。",
            );
            while self.eat_punct("[") {
                let _ = self.parse_expr();
                self.expect_punct("]");
            }
        }
        match elem {
            Some(t) => Some(Expr::NewArray {
                elem: t,
                len: Box::new(len),
            }),
            None => {
                self.diags.push(
                    Diag::error_code(
                        self.line(),
                        "JR3014",
                        "`new T[n]` 的元素类型目前只支持标量 / String",
                    )
                    .with_hint(
                        "对象数组用 `List<Foo>` 加 `add(new Foo(...))`；Rust 的 `vec![默认值; n]` 要求元素实现 Default。",
                    ),
                );
                Some(Expr::Ident("<错误>".to_string()))
            }
        }
    }

    fn looks_like_decl(&self) -> bool {
        match self.peek() {
            Tok::Kw(Kw::Int)
            | Tok::Kw(Kw::Long)
            | Tok::Kw(Kw::Double)
            | Tok::Kw(Kw::Float)
            | Tok::Kw(Kw::Boolean)
            | Tok::Kw(Kw::Char) => true,
            Tok::Ident(name) => {
                if matches!(
                    name.as_str(),
                    "String" | "List" | "ArrayList" | "Collection" | "LinkedList"
                ) {
                    true
                } else {
                    matches!(self.peek_at(1), Tok::Ident(_) | Tok::Punct("<"))
                        // `Foo[] x`：只有 `[` 紧跟 `]` 才是数组类型，`a[0]` 是下标。
                        || (matches!(self.peek_at(1), Tok::Punct("["))
                            && matches!(self.peek_at(2), Tok::Punct("]")))
                        || (matches!(self.peek_at(1), Tok::Punct("?"))
                            && matches!(self.peek_at(2), Tok::Ident(_)))
                }
            }
            _ => false,
        }
    }

    fn parse_let_stmt(&mut self, line: usize) -> Option<Stmt> {
        let ty = self.parse_type()?;
        let name = self.expect_ident();
        if !self.eat_punct("=") {
            self.err_hint(
                "变量声明必须初始化",
                "Rust 的 let 需要初值：`let x: i32 = 0;`。",
            );
            self.skip_to_stmt_end();
            return None;
        }
        let init = self.parse_expr();
        self.expect_punct(";");
        Some(Stmt::Let {
            ty,
            name,
            init,
            line,
        })
    }

    fn parse_expr_stmt(&mut self, line: usize) -> Option<Stmt> {
        let expr = self.parse_expr();
        if self.eat_punct("=") {
            let value = self.parse_expr();
            self.expect_punct(";");
            return Some(Stmt::Assign {
                target: expr,
                value,
                line,
            });
        }
        if let Some(op) = self.take_compound_op() {
            let rhs = self.parse_expr();
            self.expect_punct(";");
            let value = Expr::Binary {
                op,
                lhs: Box::new(expr.clone()),
                rhs: Box::new(rhs),
            };
            return Some(Stmt::Assign {
                target: expr,
                value,
                line,
            });
        }
        if self.check_punct("++") || self.check_punct("--") {
            self.diags.push(
                Diag::error(line, "jrust 不支持 ++ / --").with_hint("写成 `x += 1` 或 `x = x + 1`。"),
            );
            self.bump();
            self.eat_punct(";");
            return None;
        }
        if !self.eat_punct(";") {
            self.err("语句末尾缺少 `;`");
            self.skip_to_stmt_end();
            return None;
        }
        Some(Stmt::Expr { expr, line })
    }

    fn take_compound_op(&mut self) -> Option<BinOp> {
        let op = match self.peek() {
            Tok::Punct("+=") => BinOp::Add,
            Tok::Punct("-=") => BinOp::Sub,
            Tok::Punct("*=") => BinOp::Mul,
            Tok::Punct("/=") => BinOp::Div,
            Tok::Punct("%=") => BinOp::Rem,
            _ => return None,
        };
        self.bump();
        Some(op)
    }

    // ---------- 表达式 ----------

    fn parse_expr(&mut self) -> Expr {
        self.parse_or()
    }

    fn parse_or(&mut self) -> Expr {
        let mut e = self.parse_and();
        while self.check_punct("||") {
            self.bump();
            let r = self.parse_and();
            e = Expr::Binary {
                op: BinOp::Or,
                lhs: Box::new(e),
                rhs: Box::new(r),
            };
        }
        e
    }

    fn parse_and(&mut self) -> Expr {
        let mut e = self.parse_equality();
        while self.check_punct("&&") {
            self.bump();
            let r = self.parse_equality();
            e = Expr::Binary {
                op: BinOp::And,
                lhs: Box::new(e),
                rhs: Box::new(r),
            };
        }
        e
    }

    fn parse_equality(&mut self) -> Expr {
        let mut e = self.parse_relational();
        loop {
            let op = match self.peek() {
                Tok::Punct("==") => BinOp::Eq,
                Tok::Punct("!=") => BinOp::Ne,
                _ => break,
            };
            self.bump();
            let r = self.parse_relational();
            e = Expr::Binary {
                op,
                lhs: Box::new(e),
                rhs: Box::new(r),
            };
        }
        e
    }

    fn parse_relational(&mut self) -> Expr {
        let mut e = self.parse_additive();
        loop {
            let op = match self.peek() {
                Tok::Punct("<") => BinOp::Lt,
                Tok::Punct("<=") => BinOp::Le,
                Tok::Punct(">") => BinOp::Gt,
                Tok::Punct(">=") => BinOp::Ge,
                Tok::Kw(Kw::Instanceof) => {
                    self.diags.push(
                        Diag::error(self.line(), "jrust 不支持 instanceof").with_hint(
                            "Rust 用 enum + match 表达「是哪一种」，多态行为交给 trait 方法，不要运行时判类型。",
                        ),
                    );
                    self.bump();
                    let _ = self.parse_type();
                    return e;
                }
                _ => break,
            };
            self.bump();
            let r = self.parse_additive();
            e = Expr::Binary {
                op,
                lhs: Box::new(e),
                rhs: Box::new(r),
            };
        }
        e
    }

    fn parse_additive(&mut self) -> Expr {
        let mut e = self.parse_multiplicative();
        loop {
            let op = match self.peek() {
                Tok::Punct("+") => BinOp::Add,
                Tok::Punct("-") => BinOp::Sub,
                _ => break,
            };
            self.bump();
            let r = self.parse_multiplicative();
            e = Expr::Binary {
                op,
                lhs: Box::new(e),
                rhs: Box::new(r),
            };
        }
        e
    }

    fn parse_multiplicative(&mut self) -> Expr {
        let mut e = self.parse_unary();
        loop {
            let op = match self.peek() {
                Tok::Punct("*") => BinOp::Mul,
                Tok::Punct("/") => BinOp::Div,
                Tok::Punct("%") => BinOp::Rem,
                _ => break,
            };
            self.bump();
            let r = self.parse_unary();
            e = Expr::Binary {
                op,
                lhs: Box::new(e),
                rhs: Box::new(r),
            };
        }
        e
    }

    fn parse_unary(&mut self) -> Expr {
        if self.check_punct("!") {
            self.bump();
            let e = self.parse_unary();
            return Expr::Unary {
                op: UnOp::Not,
                expr: Box::new(e),
            };
        }
        if self.check_punct("-") {
            self.bump();
            let e = self.parse_unary();
            return Expr::Unary {
                op: UnOp::Neg,
                expr: Box::new(e),
            };
        }
        if self.check_punct("+") {
            self.bump();
            return self.parse_unary();
        }
        if self.check_kw(Kw::Move) {
            self.bump();
            let e = self.parse_unary();
            return Expr::Unary {
                op: UnOp::Move,
                expr: Box::new(e),
            };
        }
        if self.check_kw(Kw::Await) {
            self.bump();
            let e = self.parse_unary();
            return Expr::Await(Box::new(e));
        }
        self.parse_postfix()
    }

    fn parse_postfix(&mut self) -> Expr {
        let mut e = self.parse_primary();
        loop {
            if self.check_punct(".") {
                self.bump();
                let name = self.expect_ident();
                if self.check_punct("(") {
                    let args = self.parse_args();
                    e = Expr::Call {
                        recv: Some(Box::new(e)),
                        name,
                        args,
                    };
                } else {
                    e = Expr::Field(Box::new(e), name);
                }
            } else if self.check_punct("[") {
                self.bump();
                let idx = self.parse_expr();
                self.expect_punct("]");
                e = Expr::Index(Box::new(e), Box::new(idx));
            } else if self.check_punct("?") {
                self.bump();
                e = Expr::Try(Box::new(e));
            } else if self.check_punct("++") || self.check_punct("--") {
                self.diags.push(
                    Diag::error(self.line(), "jrust 不支持 ++ / --")
                        .with_hint("写成 `x += 1`，Rust 里就是 `x = x + 1`。"),
                );
                self.bump();
            } else {
                break;
            }
        }
        e
    }

    fn parse_args(&mut self) -> Vec<Expr> {
        let mut v = Vec::new();
        if !self.expect_punct("(") {
            return v;
        }
        if self.eat_punct(")") {
            return v;
        }
        loop {
            v.push(self.parse_expr());
            if self.eat_punct(",") {
                continue;
            }
            break;
        }
        self.expect_punct(")");
        v
    }

    /// 从当前 `(` 扫到配对的 `)`，看后面是否紧跟 `->`，以此区分闭包和普通括号表达式。
    fn is_closure_paren(&self) -> bool {
        let mut depth = 0i32;
        let mut i = self.pos;
        while i < self.toks.len() {
            match &self.toks[i].tok {
                Tok::Punct("(") => depth += 1,
                Tok::Punct(")") => {
                    depth -= 1;
                    if depth <= 0 {
                        return matches!(
                            self.toks.get(i + 1).map(|t| &t.tok),
                            Some(Tok::Punct("->"))
                        );
                    }
                }
                Tok::Eof => return false,
                _ => {}
            }
            i += 1;
        }
        false
    }

    fn parse_closure_paren(&mut self) -> Expr {
        self.expect_punct("(");
        let mut params = Vec::new();
        if !self.eat_punct(")") {
            loop {
                let ty = if self.looks_like_typed_binding() {
                    self.parse_type()
                } else {
                    None
                };
                let name = self.expect_ident();
                params.push(ClosureParam { ty, name });
                if self.eat_punct(",") {
                    continue;
                }
                break;
            }
            self.expect_punct(")");
        }
        self.expect_punct("->");
        if self.check_punct("{") {
            self.err_hint(
                "闭包体先只支持单个表达式",
                "把多行逻辑写进普通方法；闭包写成 `(a, b) -> a + b`。",
            );
            self.skip_block();
            return Expr::Closure {
                params,
                body: Box::new(Expr::Ident("()".to_string())),
            };
        }
        let body = self.parse_expr();
        Expr::Closure {
            params,
            body: Box::new(body),
        }
    }

    fn parse_primary(&mut self) -> Expr {
        match self.peek().clone() {
            Tok::Raw(code) => {
                let line = self.line();
                self.reject_nested_raw(&code, line);
                self.bump();
                Expr::Raw { code, line }
            }
            Tok::Kw(Kw::Raw) => {
                self.err_hint(
                    "raw 必须写成 `raw { ... }`",
                    "在 `raw` 后紧跟花括号，里面直接写 Rust 代码。",
                );
                self.bump();
                Expr::Int(0)
            }
            Tok::Int(n) => {
                self.bump();
                Expr::Int(n)
            }
            Tok::Float(f) => {
                self.bump();
                Expr::Float(f)
            }
            Tok::Str(s) => {
                self.bump();
                Expr::Str(s)
            }
            Tok::CharLit(c) => {
                self.bump();
                Expr::Char(c)
            }
            Tok::Kw(Kw::True) => {
                self.bump();
                Expr::Bool(true)
            }
            Tok::Kw(Kw::False) => {
                self.bump();
                Expr::Bool(false)
            }
            Tok::Kw(Kw::This) => {
                let line = self.toks[self.pos].line;
                self.bump();
                Expr::This(line)
            }
            Tok::Kw(Kw::Null) => {
                self.bump();
                Expr::Null
            }
            Tok::Kw(Kw::New) => {
                self.bump();
                if let Some(e) = self.try_new_array() {
                    return e;
                }
                let class = self.expect_ident();
                let type_args = if self.check_punct("<") {
                    self.parse_generic_args()
                } else {
                    Vec::new()
                };
                if self.check_punct("[") {
                    self.diags.push(
                        Diag::error(self.line(), "jrust MVP 不支持 new 数组").with_hint(
                            "用 `Vec::new()` 加 `push`，或直接用 `vec![...]`。",
                        ),
                    );
                    // 跳过 [ ... ]
                    let mut depth = 0i32;
                    while !self.at_eof() {
                        if self.check_punct("[") {
                            depth += 1;
                            self.bump();
                        } else if self.check_punct("]") {
                            depth -= 1;
                            self.bump();
                            if depth <= 0 {
                                break;
                            }
                        } else {
                            self.bump();
                        }
                    }
                    Expr::Ident("Vec::new()".to_string())
                } else {
                    let args = self.parse_args();
                    Expr::New {
                        class,
                        type_args,
                        args,
                    }
                }
            }
            Tok::Ident(n) => {
                self.bump();
                // 单参数闭包：`x -> x * 2`
                if matches!(self.peek(), Tok::Punct("->")) {
                    self.bump();
                    let body = self.parse_expr();
                    return Expr::Closure {
                        params: vec![ClosureParam {
                            ty: None,
                            name: n,
                        }],
                        body: Box::new(body),
                    };
                }
                // 裸调用：`println(x)` / `panic("…")` / `assert(cond, "…")` / 同文件函数。
                if self.check_punct("(") {
                    let args = self.parse_args();
                    Expr::Call {
                        recv: None,
                        name: n,
                        args,
                    }
                } else {
                    Expr::Ident(n)
                }
            }
            Tok::Punct("(") if self.is_closure_paren() => self.parse_closure_paren(),
            Tok::Punct("(") => {
                self.bump();
                let e = self.parse_expr();
                self.expect_punct(")");
                Expr::Paren(Box::new(e))
            }
            Tok::Punct("@") => {
                self.err_hint(
                    "表达式位置不支持注解",
                    "注解写在类、字段、方法或构造器的声明前。",
                );
                self.parse_annotations();
                Expr::Ident("<注解>".to_string())
            }
            _ => {
                let d = self.describe();
                self.err(format!("这里期望一个表达式，实际是 {}", d));
                self.bump();
                Expr::Ident("<错误>".to_string())
            }
        }
    }
}

/// `raw` 块内容里是否又出现 `raw {`（JR-X3）。
///
/// 跳过字符串字面量与 `//` / `/* */` 注释；`raw` 作为标识符（如 `fn raw() {`）不算嵌套，
/// 只有紧跟 `{` 的独立 `raw` 单词才算。
fn contains_nested_raw(code: &str) -> bool {
    let cs: Vec<char> = code.chars().collect();
    let n = cs.len();
    let mut i = 0;
    while i < n {
        let c = cs[i];
        // 字符串字面量
        if c == '"' {
            i += 1;
            while i < n {
                if cs[i] == '\\' {
                    i += 2;
                    continue;
                }
                if cs[i] == '"' {
                    i += 1;
                    break;
                }
                i += 1;
            }
            continue;
        }
        // 行注释
        if c == '/' && i + 1 < n && cs[i + 1] == '/' {
            i += 2;
            while i < n && cs[i] != '\n' {
                i += 1;
            }
            continue;
        }
        // 块注释（不处理嵌套注释，Rust 里极少）
        if c == '/' && i + 1 < n && cs[i + 1] == '*' {
            i += 2;
            while i + 1 < n && !(cs[i] == '*' && cs[i + 1] == '/') {
                i += 1;
            }
            i = (i + 2).min(n);
            continue;
        }
        // 标识符
        if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < n && (cs[i].is_alphanumeric() || cs[i] == '_') {
                i += 1;
            }
            let word: String = cs[start..i].iter().collect();
            if word == "raw" {
                let mut j = i;
                while j < n && cs[j].is_whitespace() {
                    j += 1;
                }
                if cs.get(j) == Some(&'{') {
                    return true;
                }
            }
            continue;
        }
        i += 1;
    }
    false
}

fn tok_text(t: &Tok) -> String {
    match t {
        Tok::Ident(s) => s.clone(),
        Tok::Int(n) => n.to_string(),
        Tok::Float(f) => format!("{:?}", f),
        Tok::Str(s) => format!("{:?}", s),
        Tok::CharLit(c) => format!("{:?}", c),
        Tok::Kw(k) => kw_text(*k).to_string(),
        Tok::Punct(p) => p.to_string(),
        Tok::Raw(s) => s.clone(),
        Tok::Doc(s) => format!("///{}", s),
        Tok::Eof => String::new(),
    }
}

fn kw_text(k: Kw) -> &'static str {
    match k {
        Kw::Class => "class",
        Kw::Interface => "interface",
        Kw::Implements => "implements",
        Kw::Extends => "extends",
        Kw::Public => "public",
        Kw::Private => "private",
        Kw::Protected => "protected",
        Kw::Static => "static",
        Kw::Final => "final",
        Kw::Void => "void",
        Kw::Return => "return",
        Kw::If => "if",
        Kw::Else => "else",
        Kw::While => "while",
        Kw::For => "for",
        Kw::New => "new",
        Kw::This => "this",
        Kw::Move => "move",
        Kw::True => "true",
        Kw::False => "false",
        Kw::Null => "null",
        Kw::Import => "import",
        Kw::Package => "package",
        Kw::Int => "int",
        Kw::Long => "long",
        Kw::Double => "double",
        Kw::Float => "float",
        Kw::Boolean => "boolean",
        Kw::Char => "char",
        Kw::Break => "break",
        Kw::Continue => "continue",
        Kw::Synchronized => "synchronized",
        Kw::Try => "try",
        Kw::Catch => "catch",
        Kw::Finally => "finally",
        Kw::Throw => "throw",
        Kw::Throws => "throws",
        Kw::Instanceof => "instanceof",
        Kw::Enum => "enum",
        Kw::Switch => "switch",
        Kw::Case => "case",
        Kw::Default => "default",
        Kw::Async => "async",
        Kw::Await => "await",
        Kw::Spawn => "spawn",
        Kw::Raw => "raw",
    }
}
