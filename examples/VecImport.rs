// 由 jrust 从 examples/VecImport.jrust 生成。请对照 .jrust 源码阅读，不要直接改这个文件。
// 每段代码前的 `// jrust:L<行号>` 指向 .jrust 源码行。
#![allow(dead_code, unused_variables, unused_mut, unused_imports)]
use std::vec::Vec;

// jrust:L5
fn main() {
    // jrust:L7
    let mut v: Vec<i32> = Vec::new();
    // jrust:L8
    v.push(1);
    // jrust:L9
    v.push(2);
    // jrust:L10
    println!("size={}", v.len());
    // jrust:L11
    println!("{}", v[0]);
}

