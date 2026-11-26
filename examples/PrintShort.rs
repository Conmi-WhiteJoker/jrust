// 由 jrust 从 examples/PrintShort.jrust 生成。请对照 .jrust 源码阅读，不要直接改这个文件。
// 每段代码前的 `// jrust:L<行号>` 指向 .jrust 源码行。
#![allow(dead_code, unused_variables, unused_mut, unused_imports)]
use std::collections::HashMap;
use std::collections::HashSet;

// jrust:L2
fn main() {
    // jrust:L3
    print!("a");
    // jrust:L4
    print!("b");
    // jrust:L5
    println!();
    // jrust:L6
    println!("Hello, jrust!");
    // jrust:L7
    println!("{}", 1 + 2);
    // jrust:L8
    let n: i32 = 7;
    // jrust:L9
    println!("n = {}", n);
    // jrust:L10
    println!("{} + {} = {}", 1, 2, 3);
    // jrust:L11
    println!("literal braces: {{}} = {}", 42);
    // jrust:L12
    println!("System.out 的老写法仍可用");
}
