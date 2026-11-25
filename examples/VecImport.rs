// 由 jrust 从 examples/VecImport.jrust 生成。请对照 .jrust 源码阅读，不要直接改这个文件。
// 每段代码前的 `// jrust:L<行号>` 指向 .jrust 源码行。
#![allow(dead_code, unused_variables, unused_mut, unused_imports)]
use std::vec::Vec;
use std::collections::HashMap;

// jrust:L5
fn main() {
    // jrust:L7
    let mut v: Vec<i32> = Vec::new();
    // jrust:L8
    v.push(1);
    // jrust:L9
    v.push(2);
    // jrust:L10
    v.push(3);
    // jrust:L11
    println!("size={}", v.len());
    // jrust:L12
    println!("v[1]={}", v[1]);
    // jrust:L15
    let mut sum: i32 = 0;
    // jrust:L16
    for x in &v {
        // jrust:L17
        sum = sum + x;
    }
    // jrust:L19
    println!("sum={}", sum);
    // jrust:L22
    let mut names: Vec<String> = Vec::new();
    // jrust:L23
    names.push(("alpha").to_string());
    // jrust:L24
    names.push(("beta").to_string());
    // jrust:L25
    println!("{} {}", names[0], names[1]);
    // jrust:L28
    let mut also: Vec<String> = Vec::new();
    // jrust:L29
    also.push(("same").to_string());
    // jrust:L30
    println!("{}", also[0]);
}

