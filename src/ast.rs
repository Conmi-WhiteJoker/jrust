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
    Array(Box<Type>),
    /// 用户自定义类型（class 或 interface 的名字）
    Named(String),
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

#[derive(Debug, Clone)]
pub struct Method {
    pub vis: Vis,
    pub is_static: bool,
    pub name: String,
    pub ret: Type,
    pub params: Vec<Param>,
    pub body: Vec<Stmt>,
    pub line: usize,
}

#[derive(Debug, Clone)]
pub struct Constructor {
    pub vis: Vis,
    pub params: Vec<Param>,
    pub body: Vec<Stmt>,
    pub line: usize,
}

#[derive(Debug, Clone)]
pub struct Field {
    pub vis: Vis,
    pub ty: Type,
    pub name: String,
    pub line: usize,
}

#[derive(Debug, Clone)]
pub struct ClassDecl {
    pub name: String,
    pub implements: Vec<String>,
    pub fields: Vec<Field>,
    pub ctor: Option<Constructor>,
    pub methods: Vec<Method>,
    pub line: usize,
}

#[derive(Debug, Clone)]
pub struct MethodSig {
    pub name: String,
    pub ret: Type,
    pub params: Vec<Param>,
    pub line: usize,
}

#[derive(Debug, Clone)]
pub struct InterfaceDecl {
    pub name: String,
    pub methods: Vec<MethodSig>,
    pub line: usize,
}

#[derive(Debug, Clone, Default)]
pub struct Program {
    pub imports: Vec<String>,
    pub interfaces: Vec<InterfaceDecl>,
    pub classes: Vec<ClassDecl>,
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
    Break {
        line: usize,
    },
    Continue {
        line: usize,
    },
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
pub enum Expr {
    Int(i64),
    Float(f64),
    Str(String),
    Char(char),
    Bool(bool),
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
        args: Vec<Expr>,
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
}
