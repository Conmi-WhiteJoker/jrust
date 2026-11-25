// 由 jrust 从 examples/MoveEdges.jrust 生成。请对照 .jrust 源码阅读，不要直接改这个文件。
// 每段代码前的 `// jrust:L<行号>` 指向 .jrust 源码行。
#![allow(dead_code, unused_variables, unused_mut, unused_imports)]
use std::collections::HashMap;

// jrust:L3
#[derive(Clone)]
pub struct Coupon {
    // jrust:L4
    pub label: String,
}

impl Coupon {
    // jrust:L6
    pub fn new(label: String) -> Self {
        Self { label }
    }
    // jrust:L10
    pub fn pass(&self, s: String) -> String {
        // jrust:L11
        return s;
    }
}

// jrust:L16
fn main() {
    // jrust:L18
    let a: String = "hello".to_string();
    // jrust:L19
    let mut v: Vec<String> = Vec::new();
    // jrust:L20
    v.push((a).to_string());
    // jrust:L21
    // jrust 提示: 显式 `move` —— 所有权从 `v` 搬到 `w`，不复制。
    let w: Vec<String> = v;
    // jrust:L24
    let k: String = "k".to_string();
    // jrust:L25
    let c: Coupon = Coupon::new(k);
    // jrust:L26
    // jrust 提示: 显式 `move` —— 所有权从 `c` 搬到 `d`，不复制。
    let d: Coupon = c;
    // jrust:L29
    let out: String = "out".to_string();
    // jrust:L30
    let got: String = d.pass(out);
    // jrust:L33
    let label: String = d.label;
    // jrust:L36
    let mut s: String = "s".to_string();
    // jrust:L37
    s = label;
    // jrust:L39
    println!("{} {}", s, got);
    // jrust:L40
    println!("{}", w.len());
}

