// 由 jrust 从 examples/RustTypes.jrust 生成。请对照 .jrust 源码阅读，不要直接改这个文件。
// 每段代码前的 `// jrust:L<行号>` 指向 .jrust 源码行。
#![allow(dead_code, unused_variables, unused_mut, unused_imports)]

// jrust:L5
fn main() {
    // jrust:L6
    let a: i32 = 40;
    // jrust:L7
    let b: i32 = 2;
    // jrust:L8
    let sum: i32 = a + b;
    // jrust:L9
    let thousand: i32 = 1000;
    // jrust:L10
    let big: i64 = 9000000000;
    // jrust:L11
    let huge: i128 = 1234567890123;
    // jrust:L12
    let small: u8 = 200;
    // jrust:L13
    let count: usize = 3;
    // jrust:L14
    let ratio: f64 = 1.5;
    // jrust:L15
    let tiny: f32 = 0.5;
    // jrust:L16
    let ok: bool = true;
    // jrust:L19
    println!("{}", sum);
    // jrust:L20
    println!("{}", thousand);
    // jrust:L21
    println!("{}", big);
    // jrust:L22
    println!("{}", huge);
    // jrust:L23
    println!("{}", small);
    // jrust:L24
    println!("{}", count);
    // jrust:L25
    println!("{}", ratio * 2.0);
    // jrust:L26
    println!("{}", tiny + 0.25);
    // jrust:L27
    println!("{}", ok);
}

