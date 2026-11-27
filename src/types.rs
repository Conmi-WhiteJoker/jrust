//! V2 类型信息层的骨架（cairn 设计文档 §14）。
//!
//! 分三个阶段落地，本文件按阶段逐步启用：
//!
//! - **阶段 A（当前）**：`TypeEnv` 上线，替换 `codegen::Env` 里那张扁平的
//!   `vars: HashMap<String, Type>`，顺手修掉块作用域缺失（循环变量 / if 分支里
//!   声明的变量会泄漏到外层）。此时 `TypeEnv` 内部仍存 `ast::Type`，**产物必须
//!   逐字节不变**（由 `tests/snapshot.sh --check` 守住）。
//! - **阶段 B/C**：`SymbolTable` 上线做名称解析，`Ty` 接管 `ast::Type` 成为
//!   codegen 的类型载体（届时 `TypeEnv` 的存储类型从 `ast::Type` 换成 `Ty`）。
//!
//! 为什么 `Ty` 现在不接入：`Ty` 比 `ast::Type` 严格（区分「已解析类型 / 类型变量 /
//! 未知」），而 codegen 目前处处依赖 `ast::Type` 的宽松表示。若在阶段 A 就换，
//! 需要大量 `Ty -> ast::Type` 的反向转换，而 `Ty::Unknown` 等变体在 `ast::Type`
//! 里没有对应物——那不是"行为不变"，是重新设计。所以骨架先立着，等阶段 C 的
//! 名称解析能保证「查到的类型一定可解析」时再切换。

#![allow(dead_code)] // 阶段 A 只启用 TypeEnv；Ty / SymbolTable 在阶段 B/C 接入。

use std::collections::HashMap;

use crate::ast;

/// 内部类型表示（阶段 C 起接管 `ast::Type`）。
///
/// 与 `ast::Type` 的关键差异：`TyVar` 与 `Named` 分开（`ast::Type` 里 `Named("T")`
/// 既可能是泛型参数也可能是用户类，无法区分），且 `Unknown` 是**显式**的——
/// 只允许表示「外部 crate 的类型」，不允许再出现"推断失败就 None"。
#[derive(Clone, Debug, PartialEq)]
pub enum Ty {
    /// 标量：`i32` / `i64` / `f64` / `bool` / `char` 等。
    Scalar(Scalar),
    /// `String`（拥有所有权的字符串；借用形态由借用推断层决定，不在类型层）。
    Str,
    List(Box<Ty>),
    Map(Box<Ty>, Box<Ty>),
    Set(Box<Ty>),
    Array(Box<Ty>),
    /// 已解析到具体声明（class / interface / enum），附泛型实参。
    Named(String, Vec<Ty>),
    /// 泛型参数（声明侧的 `T`）。
    TyVar(String),
    /// `T?` → `Option<T>`。
    Option(Box<Ty>),
    /// 函数类型（V2 之后才做，先留位）。
    Fn(Vec<Ty>, Box<Ty>),
    /// **显式未知**：仅用于外部 crate 的类型（本工程没有源码）。
    /// 使用处必须能报出 warning，不得静默。
    Unknown,
}

/// 标量类型。与 `ast::Type` 的 `Int` / `Long` / ... 及 `RustPrim` 一一对应。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scalar {
    I8,
    I16,
    I32,
    I64,
    I128,
    Isize,
    U8,
    U16,
    U32,
    U64,
    U128,
    Usize,
    F32,
    F64,
    Bool,
    Char,
}

impl Scalar {
    /// 加宽等级（`None` = 不可加宽：`bool` / `char`）。
    /// 与 `codegen::scalar_rank` 的规则保持一致——阶段 C 合并时以本函数为准。
    pub fn widen_rank(self) -> Option<u8> {
        match self {
            Scalar::I8 | Scalar::I16 | Scalar::I32 | Scalar::U8 | Scalar::U16 | Scalar::U32 => {
                Some(1)
            }
            Scalar::I64 | Scalar::Isize | Scalar::U64 | Scalar::U128 | Scalar::I128
            | Scalar::Usize => Some(2),
            Scalar::F32 => Some(3),
            Scalar::F64 => Some(4),
            Scalar::Bool | Scalar::Char => None,
        }
    }

    /// Rust 侧的写法。
    pub fn rust_name(self) -> &'static str {
        match self {
            Scalar::I8 => "i8",
            Scalar::I16 => "i16",
            Scalar::I32 => "i32",
            Scalar::I64 => "i64",
            Scalar::I128 => "i128",
            Scalar::Isize => "isize",
            Scalar::U8 => "u8",
            Scalar::U16 => "u16",
            Scalar::U32 => "u32",
            Scalar::U64 => "u64",
            Scalar::U128 => "u128",
            Scalar::Usize => "usize",
            Scalar::F32 => "f32",
            Scalar::F64 => "f64",
            Scalar::Bool => "bool",
            Scalar::Char => "char",
        }
    }
}

/// 局部类型环境：**带作用域**的变量 → 类型映射。
///
/// 替换 `codegen::Env` 里那张扁平 `HashMap`。扁平的后果是块作用域失效：
///
/// ```text
/// int x = 1;
/// if (c) { String x = "s"; }   // 内层 x 覆盖外层
/// println(x);                  // ← 这里会误认为 x 是 String
/// ```
///
/// `push_scope` / `pop_scope` 由 codegen 在生成 `{}` 块时成对调用。
#[derive(Clone, Debug, Default)]
pub struct TypeEnv {
    /// 最外层是函数体，内层是嵌套块。查找从末尾（最内层）向前。
    scopes: Vec<HashMap<String, ast::Type>>,
}

impl TypeEnv {
    /// 新建一个已含函数体外层的环境。
    pub fn new() -> Self {
        Self {
            scopes: vec![HashMap::new()],
        }
    }

    /// 进入一个块作用域。
    pub fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    /// 离开块作用域。最外层不弹出（防止误调用把函数体作用域丢掉）。
    pub fn pop_scope(&mut self) {
        if self.scopes.len() > 1 {
            self.scopes.pop();
        }
    }

    /// 在当前（最内层）作用域声明一个变量。同名则覆盖本层。
    pub fn declare(&mut self, name: impl Into<String>, ty: ast::Type) {
        if let Some(top) = self.scopes.last_mut() {
            top.insert(name.into(), ty);
        }
    }

    /// 从最内层向外查找。
    pub fn lookup(&self, name: &str) -> Option<&ast::Type> {
        self.scopes.iter().rev().find_map(|s| s.get(name))
    }

    /// 是否存在该名字（任意层）。
    pub fn contains(&self, name: &str) -> bool {
        self.lookup(name).is_some()
    }

    /// 当前可见的全部变量名（内层遮蔽外层：同一名字只出现一次）。
    ///
    /// `spawn` 的自由变量分析需要这个视图——它关心「闭包会捕获哪些名字」，
    /// 而不关心某个名字来自哪一层。
    pub fn visible_names(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for scope in self.scopes.iter().rev() {
            for k in scope.keys() {
                if !out.contains(k) {
                    out.push(k.clone());
                }
            }
        }
        out
    }

    /// 当前作用域层数（调试与断言用）。
    pub fn depth(&self) -> usize {
        self.scopes.len()
    }
}

/// 名称解析表（阶段 B 起启用）。
///
/// 回答「这个名字是什么」：类型位置 → class / interface / enum；表达式位置 →
/// 局部变量 / 形参 / 字段 / 静态常量；`recv.name` → 按 `recv` 的静态类型查成员。
///
/// 阶段 A 只立骨架；`codegen` 仍用自己的 `classes` / `ifaces` / `enums` 索引。
#[derive(Clone, Debug, Default)]
pub struct SymbolTable {
    classes: HashMap<String, ClassInfo>,
    ifaces: HashMap<String, IfaceInfo>,
    enums: HashMap<String, EnumInfo>,
}

/// 类的符号信息：泛型参数、字段表、方法表。
#[derive(Clone, Debug, Default)]
pub struct ClassInfo {
    pub name: String,
    pub generics: Vec<String>,
    pub fields: HashMap<String, ast::Type>,
    pub methods: HashMap<String, MethodSig>,
    /// 实现的接口名（`implements` 列表）。
    pub implements: Vec<String>,
}

/// 接口的符号信息。
#[derive(Clone, Debug, Default)]
pub struct IfaceInfo {
    pub name: String,
    pub generics: Vec<String>,
    pub methods: HashMap<String, MethodSig>,
    /// `interface A extends B, C` 的父接口。
    pub extends: Vec<String>,
}

/// 枚举的符号信息：分支名 → 载荷类型列表。
#[derive(Clone, Debug, Default)]
pub struct EnumInfo {
    pub name: String,
    pub variants: HashMap<String, Vec<ast::Type>>,
}

/// 方法签名（名称解析阶段的粗粒度版本；借用三态另由 `analysis::Sigs` 提供）。
#[derive(Clone, Debug, Default)]
pub struct MethodSig {
    pub is_static: bool,
    pub params: Vec<ast::Type>,
    pub ret: Option<ast::Type>,
}

impl SymbolTable {
    pub fn class(&self, name: &str) -> Option<&ClassInfo> {
        self.classes.get(name)
    }

    pub fn iface(&self, name: &str) -> Option<&IfaceInfo> {
        self.ifaces.get(name)
    }

    pub fn enm(&self, name: &str) -> Option<&EnumInfo> {
        self.enums.get(name)
    }

    /// 该名字是否是一个已声明的类型（class / interface / enum 任一）。
    pub fn is_type(&self, name: &str) -> bool {
        self.classes.contains_key(name)
            || self.ifaces.contains_key(name)
            || self.enums.contains_key(name)
    }

    /// 按类名查字段类型。
    pub fn field(&self, class: &str, field: &str) -> Option<&ast::Type> {
        self.classes.get(class)?.fields.get(field)
    }

    /// 按类名 / 接口名查方法签名。
    pub fn method(&self, owner: &str, name: &str) -> Option<&MethodSig> {
        if let Some(c) = self.classes.get(owner) {
            if let Some(m) = c.methods.get(name) {
                return Some(m);
            }
        }
        self.ifaces.get(owner)?.methods.get(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inner_scope_shadows_outer_and_restores() {
        let mut env = TypeEnv::new();
        env.declare("x", ast::Type::Int);
        env.push_scope();
        env.declare("x", ast::Type::String);
        assert_eq!(env.lookup("x"), Some(&ast::Type::String));
        env.pop_scope();
        // 关键：离开块后必须看到外层类型，而不是被内层覆盖过的。
        assert_eq!(env.lookup("x"), Some(&ast::Type::Int));
    }

    #[test]
    fn block_local_does_not_leak() {
        let mut env = TypeEnv::new();
        env.push_scope();
        env.declare("tmp", ast::Type::Int);
        env.pop_scope();
        assert_eq!(env.lookup("tmp"), None);
    }

    #[test]
    fn pop_at_outermost_is_noop() {
        let mut env = TypeEnv::new();
        env.declare("x", ast::Type::Int);
        env.pop_scope(); // 不应丢掉函数体作用域
        assert_eq!(env.lookup("x"), Some(&ast::Type::Int));
        assert_eq!(env.depth(), 1);
    }

    #[test]
    fn visible_names_dedups_by_inner_scope() {
        let mut env = TypeEnv::new();
        env.declare("a", ast::Type::Int);
        env.declare("b", ast::Type::Int);
        env.push_scope();
        env.declare("b", ast::Type::String);
        env.declare("c", ast::Type::Int);
        let mut names = env.visible_names();
        names.sort();
        assert_eq!(names, vec!["a".to_string(), "b".to_string(), "c".to_string()]);
    }

    #[test]
    fn scalar_widen_rank_matches_codegen_rules() {
        // 与 codegen::scalar_rank 同口径：int/i32 < long/i64 < f32 < f64。
        assert!(Scalar::I32.widen_rank() < Scalar::I64.widen_rank());
        assert!(Scalar::I64.widen_rank() < Scalar::F32.widen_rank());
        assert!(Scalar::F32.widen_rank() < Scalar::F64.widen_rank());
        assert_eq!(Scalar::Bool.widen_rank(), None);
        assert_eq!(Scalar::Char.widen_rank(), None);
    }
}
