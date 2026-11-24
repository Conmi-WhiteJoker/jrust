// 由 jrust 从 examples/Ownership.jrust 生成。请对照 .jrust 源码阅读，不要直接改这个文件。
// 每段代码前的 `// jrust:L<行号>` 指向 .jrust 源码行。
#![allow(dead_code, unused_variables, unused_mut)]

// jrust:L5
fn main() {
    // jrust:L6
    let a: String = "hello".to_string();
    // jrust:L7
    // jrust 提示: `a` 是移动语义——所有权从 `a` 转给 `b`，之后再用 `a` 会被借用检查器拒绝；需要两份就写 `a.clone()`。
    let b: String = a;
    // jrust:L8
    println!("{}", b);
}

