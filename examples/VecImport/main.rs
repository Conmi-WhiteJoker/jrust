// 由 jrust 从 examples/VecImport/main.jrust 生成。请对照 .jrust 源码阅读，不要直接改这个文件。
// 每段代码前的 `// jrust:L<行号>` 指向 .jrust 源码行。
#![allow(dead_code, unused_variables, unused_mut, unused_imports)]
use std::vec::Vec;
use std::collections::HashMap;
use std::collections::HashSet;

// jrust:L8
fn main() {
    // jrust:L10
    let mut v: Vec<i32> = Vec::new();
    // jrust:L11
    v.push(1);
    // jrust:L12
    v.push(2);
    // jrust:L13
    v.push(3);
    // jrust:L14
    println!("size={}", v.len());
    // jrust:L15
    println!("v[1]={}", v[1]);
    // jrust:L18
    let mut sum: i32 = 0;
    // jrust:L19
    for x in &v {
        // jrust:L20
        sum = sum + x;
    }
    // jrust:L22
    println!("sum={}", sum);
    // jrust:L25
    let mut names: Vec<String> = Vec::new();
    // jrust:L26
    names.push(("alpha").to_string());
    // jrust:L27
    names.push(("beta").to_string());
    // jrust:L28
    println!("{} {}", names[0], names[1]);
    // jrust:L31
    let mut also: Vec<String> = Vec::new();
    // jrust:L32
    also.push(("same").to_string());
    // jrust:L33
    println!("{}", also[0]);
}
