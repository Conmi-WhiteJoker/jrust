// 由 jrust 从 examples/Exceptions/main.jrust 生成。请对照 .jrust 源码阅读，不要直接改这个文件。
// 每段代码前的 `// jrust:L<行号>` 指向 .jrust 源码行。
#![allow(dead_code, unused_variables, unused_mut, unused_imports, unused_parens, non_snake_case, unused_assignments)]
use std::collections::HashMap;
use std::collections::HashSet;

#[derive(Debug)]
pub enum JrustError {
    AppError(String),
}

impl std::fmt::Display for JrustError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JrustError::AppError(m) => write!(f, "AppError: {}", m),
        }
    }
}
impl std::error::Error for JrustError {}

impl From<AppError> for JrustError {
    fn from(e: AppError) -> Self { JrustError::AppError(e.message) }
}

// jrust:L3
#[derive(Debug, Clone)]
pub struct AppError {
    pub message: String,
}

impl AppError {
    pub fn new(message: impl Into<String>) -> Self {
        Self { message: message.into() }
    }
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "AppError: {}", self.message)
    }
}
impl std::error::Error for AppError {}

// jrust:L6
#[derive(Clone)]
pub struct Calc;

impl Calc {
    // jrust:L7
    pub fn divide(&self, a: i32, b: i32) -> Result<i32, JrustError> {
        // jrust:L8
        if b == 0 {
            // jrust:L9
            return Err(JrustError::AppError(("division by zero").to_string()));
        }
        // jrust:L11
        return Ok(a / b);
    }
    // jrust:L14
    pub fn twice(&self, a: i32, b: i32) -> Result<i32, JrustError> {
        // jrust:L15
        return Ok(self.divide(a, b)? * 2);
    }
}

// jrust:L20
fn main() -> Result<(), JrustError> {
    // jrust:L21
    let c: Calc = Calc;
    // jrust:L22
    println!("{}", c.twice(14, 2)?);
    // jrust:L23
    println!("{}", c.twice(9, 3)?);
    Ok(())
}
