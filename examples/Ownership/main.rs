// 由 jrust 从 examples/Ownership/main.jrust 生成。请对照 .jrust 源码阅读，不要直接改这个文件。
// 每段代码前的 `// jrust:L<行号>` 指向 .jrust 源码行。
#![allow(dead_code, unused_variables, unused_mut, unused_imports)]
use std::collections::HashMap;
use std::collections::HashSet;

// jrust:L6
fn main() {
    // jrust:L7
    let a: String = "hello".to_string();
    // jrust:L8
    // jrust 提示: 显式 `move` —— 所有权从 `a` 搬到 `b`，不复制。
    let b: String = a;
    // jrust:L9
    let c: String = "world".to_string();
    // jrust:L10
    // jrust 提示: 普通赋值按值复制——`c` 与 `d` 各持一份；只想转移所有权请写 `move c`。
    let d: String = c.clone();
    // jrust:L11
    println!("{} {}", b, d);
    // jrust:L13
    println!("{} {}", b, d);
}
