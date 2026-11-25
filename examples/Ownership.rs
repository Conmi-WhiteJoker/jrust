// 由 jrust 从 examples/Ownership.jrust 生成。请对照 .jrust 源码阅读，不要直接改这个文件。
// 每段代码前的 `// jrust:L<行号>` 指向 .jrust 源码行。
#![allow(dead_code, unused_variables, unused_mut)]

// jrust:L6
fn main() {
    // jrust:L7
    let a: String = "hello".to_string();
    // jrust:L8
    // jrust 提示: 显式 `move` —— 所有权从 `a` 搬到 `b`。Rust 的赋值本身就是 move，所以产物与不加 `move` 相同；这个关键字只是把意图写出来。
    let b: String = a;
    // jrust:L9
    let c: String = "world".to_string();
    // jrust:L10
    // jrust 提示: `c` 是移动语义——所有权从 `c` 转给 `d`，之后再用 `c` 会被借用检查器拒绝；需要两份就写 `c.clone()`。
    let d: String = c;
    // jrust:L11
    println!("{} {}", b, d);
    // jrust:L13
    println!("{} {}", b, d);
}

