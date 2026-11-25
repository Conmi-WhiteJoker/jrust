//! jrust 的抽象语法树。
//!
//! 这里只描述「Java 风格源码」的结构，不包含任何 Rust 概念；
//! Java 到 Rust 的映射全部集中在 `codegen` 里，方便对照阅读。

#[derive(Debug, Clone, PartialEq)]
pub enum Type {
    Int,
    Long,
    Double,
    Float,
    Boolean,
    Char,
    Void,
    String,
    /// Rust 原生标量类型名（`i32` / `u64` / `f64` / `usize` / `bool` ...），原样输出。
    RustPrim(String),
    List(Box<Type>),
    /// `Map<K, V>` / `HashMap<K, V>` → `HashMap<K, V>`
    Map(Box<Type>, Box<Type>),
    Array(Box<Type>),
    /// 用户自定义类型（class 或 interface 的名字）
    Named(String),
    /// 带类型实参的用户类型：`Box<Integer>` → `Box<i32>`。
    /// 声明侧的 `T` 仍是 `Named("T")`，只有使用处才带实参。
    Generic(String, Vec<Type>),
    /// `T?` 表示可空类型 → `Option<T>`。
    Nullable(Box<Type>),
}

impl Type {
    /// 标量 Copy 类型：赋值时复制而不是移动所有权。
    pub fn is_copy_scalar(&self) -> bool {
        matches!(
            self,
            Type::Int
                | Type::Long
                | Type::Double
                | Type::Float
                | Type::Boolean
                | Type::Char
                | Type::RustPrim(_)
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Vis {
    Public,
    Private,
    Package,
}

#[derive(Debug, Clone)]
pub struct Param {
    pub ty: Type,
    pub name: String,
}

/// 声明侧类型参数：`<T extends A & B>` 里的 `T` 与上界 `A`、`B`。
///
/// 上界只能是 interface（→ Rust trait）；`impl` / 方法位置输出时还会追加 `Clone`，
/// 因为泛型上下文里的赋值按值语义补 `.clone()`。
#[derive(Debug, Clone)]
pub struct TypeParam {
    pub name: String,
    pub bounds: Vec<Type>,
}

/// `@Xxx(args)` 注解（JR-D9）。
///
/// `@derive(A, B)` 合并进 `#[derive(...)]`；`@raw("…")` 把字符串内容原样输出；
/// 其余按同名 Rust 属性透传（`@inline` → `#[inline]`）。
#[derive(Debug, Clone)]
pub struct Attr {
    pub name: String,
    /// 括号内实参的原始 token 文本拼接（无参时为 `None`）。
    pub args: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Method {
    pub vis: Vis,
    pub is_static: bool,
    /// `async` 修饰符：`pub async fn`（JR-C9）。
    pub is_async: bool,
    /// 方法上的注解（→ `#[...]`）。
    pub attrs: Vec<Attr>,
    /// 方法级类型参数：`public <T> T id(T x)` 里的 `T`。
    pub type_params: Vec<TypeParam>,
    pub name: String,
    pub ret: Type,
    pub params: Vec<Param>,
    /// `throws A, B` 里的异常类型名（可带包路径），codegen 用它生成错误枚举分支。
    pub throws: Vec<String>,
    pub body: Vec<Stmt>,
    pub line: usize,
}

#[derive(Debug, Clone)]
pub struct Constructor {
    pub vis: Vis,
    pub attrs: Vec<Attr>,
    pub params: Vec<Param>,
    pub body: Vec<Stmt>,
    pub line: usize,
}

#[derive(Debug, Clone)]
pub struct Field {
    pub vis: Vis,
    /// `static final` 字段：→ 模块级 `pub const`，不进入结构体。
    pub is_static: bool,
    pub is_final: bool,
    /// 仅 `static final` 字段有初始值（Java 静态常量必须带初值）。
    pub init: Option<Expr>,
    pub attrs: Vec<Attr>,
    pub ty: Type,
    pub name: String,
    pub line: usize,
}

#[derive(Debug, Clone)]
pub struct ClassDecl {
    pub name: String,
    pub attrs: Vec<Attr>,
    /// 类级类型参数：`class Box<T>` 里的 `T`。
    pub type_params: Vec<TypeParam>,
    /// `implements A, B<Integer>`：`Named` 或 `Generic`。
    pub implements: Vec<Type>,
    pub fields: Vec<Field>,
    pub ctor: Option<Constructor>,
    pub methods: Vec<Method>,
    /// 类内的 `raw { ... }`（JR-X1）：作为顶层条目原样输出。
    pub raw_members: Vec<RawCode>,
    pub line: usize,
}

/// `raw { ... }`：原样内联的 Rust 源码（JR-X1）。
#[derive(Debug, Clone)]
pub struct RawCode {
    pub code: String,
    pub line: usize,
}

#[derive(Debug, Clone)]
pub struct MethodSig {
    pub attrs: Vec<Attr>,
    /// 方法级类型参数：`interface Mapper<T> { <U> U f(U x); }` 里的 `U`。
    pub type_params: Vec<TypeParam>,
    pub name: String,
    pub ret: Type,
    pub params: Vec<Param>,
    /// `static` 接口方法 → trait 关联函数（没有 `&self`）。
    pub is_static: bool,
    /// `Some` 表示默认方法（trait 默认实现）；`None` 表示只有签名。
    pub body: Option<Vec<Stmt>>,
    pub line: usize,
}

#[derive(Debug, Clone)]
pub struct InterfaceDecl {
    pub name: String,
    pub attrs: Vec<Attr>,
    /// 接口级类型参数：`interface Mapper<T>` 里的 `T`。
    pub type_params: Vec<TypeParam>,
    pub methods: Vec<MethodSig>,
    pub line: usize,
}

/// 带数据枚举的分支：`Circle(double r)`。
#[derive(Debug, Clone)]
pub struct EnumVariant {
    pub name: String,
    pub params: Vec<Param>,
    pub line: usize,
}

/// `enum Shape { Circle(double r), Rect(double w, double h), Empty }`
#[derive(Debug, Clone)]
pub struct EnumDecl {
    pub name: String,
    pub attrs: Vec<Attr>,
    /// 枚举级类型参数：`enum Maybe<T>` 里的 `T`。
    pub type_params: Vec<TypeParam>,
    pub variants: Vec<EnumVariant>,
    pub line: usize,
}

#[derive(Debug, Clone, Default)]
pub struct Program {
    pub imports: Vec<String>,
    pub interfaces: Vec<InterfaceDecl>,
    pub classes: Vec<ClassDecl>,
    pub enums: Vec<EnumDecl>,
}

#[derive(Debug, Clone)]
pub enum Stmt {
    Let {
        ty: Type,
        name: String,
        init: Expr,
        line: usize,
    },
    Assign {
        target: Expr,
        value: Expr,
        line: usize,
    },
    Expr {
        expr: Expr,
        line: usize,
    },
    Return {
        value: Option<Expr>,
        line: usize,
    },
    If {
        cond: Expr,
        then: Vec<Stmt>,
        els: Option<Vec<Stmt>>,
        line: usize,
    },
    While {
        cond: Expr,
        body: Vec<Stmt>,
        line: usize,
    },
    ForEach {
        ty: Type,
        name: String,
        iter: Expr,
        body: Vec<Stmt>,
        line: usize,
    },
    /// C 风格计数循环：`for (int i = 0; i < n; i++)` → `for i in 0..n`。
    ///
    /// 只接受「同一个计数变量 + 单比较 + 自增/自减」的形状：
    /// `i++` 配 `<` / `<=`，`i--` 配 `>` / `>=`；其它组合在 parser 报 JR2003。
    ForRange {
        ty: Type,
        name: String,
        start: Expr,
        end: Expr,
        /// 条件为 `<=` / `>=`，生成 `..=`。
        inclusive: bool,
        /// 递减循环，生成 `(a..b).rev()`。
        descending: bool,
        body: Vec<Stmt>,
        line: usize,
    },
    Break {
        line: usize,
    },
    Continue {
        line: usize,
    },
    /// `switch (x) { case ...: ... }` → Rust `match`。
    Switch {
        subject: Expr,
        cases: Vec<SwitchCase>,
        default: Option<Vec<Stmt>>,
        line: usize,
    },
    /// `spawn { ... }` / `spawn f();` → `tokio::spawn(async move { ... });`（JR-C11）。
    Spawn {
        body: Vec<Stmt>,
        line: usize,
    },
    /// `raw { ... }`（JR-X1）：语句位置原样输出。
    Raw {
        code: String,
        line: usize,
    },
    /// `throw new E("msg");` → `return Err(JrustError::E(...));`
    Throw {
        expr: Expr,
        line: usize,
    },
}

#[derive(Debug, Clone)]
pub struct SwitchCase {
    pub pattern: CasePattern,
    pub guard: Option<Expr>,
    pub body: Vec<Stmt>,
    pub line: usize,
}

#[derive(Debug, Clone)]
pub enum CasePattern {
    /// `case true:` / `case false:`
    Bool(bool),
    /// `case Circle(r):` / `case Shape.Circle(double r):`
    Variant {
        enum_name: Option<String>,
        name: String,
        bindings: Vec<String>,
    },
    /// 其它字面量（int / String 等）：Rust 的 match 无法穷尽，codegen 会报 JR3019。
    Other(Expr),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
    And,
    Or,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Not,
    /// jrust 扩展：`move x` 显式移走 x 的所有权（Rust 中赋值本身就是 move）。
    Move,
}

#[derive(Debug, Clone)]
pub struct ClosureParam {
    /// 可选类型标注：`(double x) -> x`；省略时由 Rust 推断。
    pub ty: Option<Type>,
    pub name: String,
}

#[derive(Debug, Clone)]
pub enum Expr {
    Int(i64),
    Float(f64),
    Str(String),
    Char(char),
    Bool(bool),
    /// `null`：只允许出现在可空类型上下文（`T?`），产物为 `None`。
    Null,
    Ident(String),
    This,
    Field(Box<Expr>, String),
    Call {
        recv: Option<Box<Expr>>,
        name: String,
        args: Vec<Expr>,
    },
    New {
        class: String,
        /// `new Box<Integer>(..)` 的实参；钻石 `new Box<>(..)` 为空。
        type_args: Vec<Type>,
        args: Vec<Expr>,
    },
    /// `new T[n]` → `vec![默认值; n]`；元素类型在 parser 已限定为标量 / String。
    NewArray {
        elem: Type,
        len: Box<Expr>,
    },
    Binary {
        op: BinOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
    Unary {
        op: UnOp,
        expr: Box<Expr>,
    },
    Index(Box<Expr>, Box<Expr>),
    Paren(Box<Expr>),
    /// `(a, b) -> expr` → `|a, b| expr`
    Closure {
        params: Vec<ClosureParam>,
        body: Box<Expr>,
    },
    /// `await expr` → `expr.await`（JR-C10）；只能在 `async` 方法里出现。
    Await(Box<Expr>),
    /// `raw { ... }`（JR-X1）：表达式位置原样输出。
    Raw {
        code: String,
        line: usize,
    },
    /// `f()?`：错误传播，只能出现在声明了 `throws` 的函数里。
    Try(Box<Expr>),
}
