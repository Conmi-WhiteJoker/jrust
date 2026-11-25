//! 手写递归下降语法分析器。
//!
//! 设计原则：只接受 jrust 支持的 Java 子集；遇到被砍掉的特性（继承、try/catch、
//! null、instanceof、synchronized、注解……）时给出「为什么 Rust 不这么干」的解释，
//! 而不是含糊地报「语法错误」。

use crate::ast::*;
use crate::diag::Diag;
use crate::lexer::{lex, Kw, Tok, Token};

enum TopItem {
    Class(ClassDecl),
    Interface(InterfaceDecl),
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

    fn skip_annotation(&mut self) {
        self.bump(); // @
        while matches!(self.peek(), Tok::Ident(_)) || self.check_punct(".") {
            self.bump();
        }
        if self.check_punct("(") {
            self.skip_parens();
        }
    }

    // ---------- 顶层 ----------

    fn parse_program(&mut self) -> Program {
        let mut prog = Program::default();
        while !self.at_eof() {
            let start = self.pos;
            match self.peek().clone() {
                Tok::Kw(Kw::Import) | Tok::Kw(Kw::Package) => {
                    let is_package = self.check_kw(Kw::Package);
                    let line = self.line();
                    self.bump();
                    let mut path = String::new();
                    while !self.at_eof() && !self.check_punct(";") {
                        match self.peek().clone() {
                            Tok::Ident(s) => {
                                path.push_str(&s);
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
                    if is_package {
                        self.diags.push(
                            Diag::error(line, "jrust 不需要 package 声明").with_hint(
                                "Rust 里一个 crate 就是一个包；模块用 `mod` 组织，跨包依赖写在 Cargo.toml。",
                            ),
                        );
                    } else {
                        prog.imports.push(path);
                    }
                }
                Tok::Kw(Kw::Public) | Tok::Kw(Kw::Class) | Tok::Kw(Kw::Interface) => {
                    if let Some(item) = self.parse_type_decl() {
                        match item {
                            TopItem::Class(c) => prog.classes.push(c),
                            TopItem::Interface(i) => prog.interfaces.push(i),
                        }
                    }
                }
                _ => {
                    self.err("顶层只允许 `public class` / `public interface` / `import`");
                    self.bump();
                }
            }
            if self.pos == start && !self.at_eof() {
                self.bump();
            }
        }
        prog
    }

    fn parse_type_decl(&mut self) -> Option<TopItem> {
        self.eat_kw(Kw::Public);
        if self.check_kw(Kw::Class) {
            return Some(TopItem::Class(self.parse_class()));
        }
        if self.check_kw(Kw::Interface) {
            return Some(TopItem::Interface(self.parse_interface()));
        }
        self.err("这里期望 `class` 或 `interface`");
        self.bump();
        None
    }

    fn parse_class(&mut self) -> ClassDecl {
        let line = self.line();
        self.bump(); // class
        let name = self.expect_ident();

        let prev_name = std::mem::replace(&mut self.class_name, name.clone());

        if self.check_punct("<") {
            self.err_hint(
                "jrust MVP 不支持泛型类",
                "先写具体类型；Rust 的泛型写作 `struct Foo<T>`，后续版本再支持。",
            );
            self.skip_generic_args();
        }

        let mut implements = Vec::new();
        if self.eat_kw(Kw::Extends) {
            self.diags.push(
                Diag::error(line, "jrust 不支持类继承（extends）").with_hint(
                    "Rust 没有继承：共享行为抽成 interface（→ trait），共享数据放进字段，组合优于继承。",
                ),
            );
            self.expect_ident();
            self.skip_generic_args();
        }
        if self.eat_kw(Kw::Implements) {
            loop {
                let iface = self.expect_ident();
                if self.check_punct("<") {
                    self.err_hint(
                        "jrust MVP 不支持泛型接口",
                        "先用非泛型的 interface，把类型参数改成具体类型。",
                    );
                    self.skip_generic_args();
                }
                implements.push(iface);
                if !self.eat_punct(",") {
                    break;
                }
            }
        }

        let mut class = ClassDecl {
            name: name.clone(),
            implements,
            fields: Vec::new(),
            ctor: None,
            methods: Vec::new(),
            line,
        };

        if self.expect_punct("{") {
            while !self.at_eof() && !self.check_punct("}") {
                let start = self.pos;
                self.parse_member(&mut class);
                if self.pos == start && !self.at_eof() {
                    self.bump();
                }
            }
            self.expect_punct("}");
        }

        self.class_name = prev_name;
        class
    }

    fn parse_member(&mut self, class: &mut ClassDecl) {
        // 注解：报告后跳过注解本身，方法体仍可继续转译
        while self.check_punct("@") {
            self.err_hint(
                "jrust 不支持注解（@Xxx）",
                "Rust 没有注解：需要的能力（序列化、测试等）用 derive 宏或显式代码表达。",
            );
            self.skip_annotation();
        }
        if self.at_eof() || self.check_punct("}") {
            return;
        }

        let mut vis = Vis::Package;
        let mut is_static = false;
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
            } else if self.eat_kw(Kw::Final) {
                // final 对字段是「不可变」，对方法是「不可覆写」，Rust 默认即不可变 / 无继承
            } else {
                break;
            }
        }

        // 构造器：名字 == 类名，且后面紧跟 `(`
        if let Tok::Ident(n) = self.peek().clone() {
            if n == class.name && matches!(self.peek_at(1), Tok::Punct("(")) {
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
            let params = self.parse_params();
            if self.check_kw(Kw::Throws) {
                self.diags.push(
                    Diag::error(mline, "jrust 不支持受检异常（throws）").with_hint(
                        "Rust 用 `Result<T, E>` 表达可能失败：返回类型写 Result，调用处用 `?` 传播。",
                    ),
                );
                self.skip_to_stmt_end();
            }
            let body = self.parse_block();
            class.methods.push(Method {
                vis,
                is_static,
                name,
                ret: ty,
                params,
                body,
                line: mline,
            });
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
                ty,
                name,
                line: member_line,
            });
        }
    }

    fn parse_interface(&mut self) -> InterfaceDecl {
        let line = self.line();
        self.bump(); // interface
        let name = self.expect_ident();
        if self.check_punct("<") {
            self.err_hint(
                "jrust MVP 不支持泛型接口",
                "先用具体类型的 interface。",
            );
            self.skip_generic_args();
        }
        if self.eat_kw(Kw::Extends) {
            self.diags.push(
                Diag::error(line, "jrust 不支持接口继承（extends）").with_hint(
                    "Rust 用 trait 组合：把公共方法抽到另一个 trait，类型同时实现多个 trait。",
                ),
            );
            self.expect_ident();
            self.skip_generic_args();
        }
        let mut methods = Vec::new();
        if self.expect_punct("{") {
            while !self.at_eof() && !self.check_punct("}") {
                let start = self.pos;
                let mline = self.line();
                let mut saw_static = false;
                loop {
                    if self.eat_kw(Kw::Public) || self.eat_kw(Kw::Final) {
                        continue;
                    }
                    if self.check_kw(Kw::Static) {
                        self.bump();
                        saw_static = true;
                        continue;
                    }
                    break;
                }
                if saw_static {
                    self.diags.push(
                        Diag::error(mline, "jrust MVP 不支持接口里的 static 方法").with_hint(
                            "Rust 的 trait 可以有关联函数，但初版先不映射；把工具方法放进普通类。",
                        ),
                    );
                }
                match self.parse_type() {
                    Some(ret) => {
                        let mname = self.expect_ident();
                        let params = self.parse_params();
                        self.expect_punct(";");
                        methods.push(MethodSig {
                            name: mname,
                            ret,
                            params,
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
        InterfaceDecl { name, methods, line }
    }

    // ---------- 类型 ----------

    fn parse_type(&mut self) -> Option<Type> {
        let line = self.line();
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
                        self.diags.push(
                            Diag::error(line, "jrust MVP 暂不支持 Map").with_hint(
                                "Rust 目标是 HashMap<K, V>，初版先不映射；需要时用两个 List 过渡。",
                            ),
                        );
                        if self.eat_punct("<") {
                            let _ = self.parse_type();
                            self.eat_punct(",");
                            let _ = self.parse_type();
                            self.expect_punct(">");
                        }
                        Type::Named("Map".to_string())
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

        while self.check_punct("[") && matches!(self.peek_at(1), Tok::Punct("]")) {
            self.bump();
            self.bump();
            base = Type::Array(Box::new(base));
        }
        Some(base)
    }

    fn parse_params(&mut self) -> Vec<Param> {
        let mut v = Vec::new();
        if !self.expect_punct("(") {
            return v;
        }
        if self.eat_punct(")") {
            return v;
        }
        loop {
            let ty = match self.parse_type() {
                Some(t) => t,
                None => Type::Int,
            };
            let name = self.expect_ident();
            v.push(Param { ty, name });
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

    fn parse_stmt(&mut self) -> Option<Stmt> {
        let line = self.line();
        match self.peek().clone() {
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
                } else {
                    self.diags.push(
                        Diag::error(line, "jrust 不支持 C 风格的 for(初始化; 条件; 步进)")
                            .with_hint(
                                "改用增强 for：`for (Animal a : animals)`。需要计数时用 `int i = 0; while (i < n) { ...; i += 1; }`。",
                            ),
                    );
                    self.skip_parens();
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
                self.expect_punct(";");
                Some(Stmt::Break { line })
            }
            Tok::Kw(Kw::Continue) => {
                self.bump();
                self.expect_punct(";");
                Some(Stmt::Continue { line })
            }
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
            Tok::Kw(Kw::Throw) => {
                self.diags.push(
                    Diag::error(line, "jrust 不支持 throw").with_hint(
                        "用 `return Err(...)` 返回错误，调用方用 `?` 传播。",
                    ),
                );
                self.skip_to_stmt_end();
                None
            }
            Tok::Kw(Kw::Synchronized) => {
                self.diags.push(
                    Diag::error(line, "jrust 不支持 synchronized").with_hint(
                        "Rust 把锁和数据绑在一起：`let m = Mutex::new(x); let g = m.lock().unwrap();`，锁在作用域结束时自动释放。",
                    ),
                );
                self.bump();
                if self.check_punct("(") {
                    self.skip_parens();
                }
                self.skip_body();
                None
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
                    matches!(
                        self.peek_at(1),
                        Tok::Ident(_) | Tok::Punct("<") | Tok::Punct("[")
                    )
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

    fn parse_primary(&mut self) -> Expr {
        match self.peek().clone() {
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
                self.bump();
                Expr::This
            }
            Tok::Kw(Kw::Null) => {
                self.diags.push(
                    Diag::error(self.line(), "jrust 没有 null").with_hint(
                        "用 Option<T> 表达缺失：Some(x) 表示有值，None 表示没有；编译器因此能强制你处理空值。",
                    ),
                );
                self.bump();
                Expr::Ident("None".to_string())
            }
            Tok::Kw(Kw::New) => {
                self.bump();
                let class = self.expect_ident();
                if self.check_punct("<") {
                    self.skip_generic_args();
                }
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
                    Expr::New { class, args }
                }
            }
            Tok::Ident(n) => {
                self.bump();
                // 裸 `print` / `println` 视作 System.out 的简写；其它裸调用仍不解析。
                if (n == "print" || n == "println") && self.check_punct("(") {
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
            Tok::Punct("(") => {
                self.bump();
                let e = self.parse_expr();
                self.expect_punct(")");
                Expr::Paren(Box::new(e))
            }
            Tok::Punct("@") => {
                self.err_hint(
                    "jrust 不支持注解",
                    "Rust 用 derive 宏或显式代码代替注解。",
                );
                self.skip_annotation();
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
    }
}
