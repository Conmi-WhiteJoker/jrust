// 由 jrust 从 examples/ClassInterface.jrust 生成。请对照 .jrust 源码阅读，不要直接改这个文件。
// 每段代码前的 `// jrust:L<行号>` 指向 .jrust 源码行。
#![allow(dead_code, unused_variables, unused_mut)]

// jrust:L2
pub trait Shape {
    // jrust:L3
    fn area(&self) -> f64;
    // jrust:L4
    fn name(&self) -> String;
}

// jrust:L7
pub struct Circle {
    // jrust:L8
    radius: f64,
}

impl Shape for Circle {
    // jrust:L14
    fn area(&self) -> f64 {
        // jrust:L15
        return 3.14159 * self.radius * self.radius;
    }
    // jrust:L18
    fn name(&self) -> String {
        // jrust:L19
        return "Circle".to_string();
    }
}

impl Circle {
    // jrust:L10
    pub fn new(radius: f64) -> Self {
        Self { radius }
    }
}

// jrust:L24
fn main() {
    // jrust:L25
    let s: Box<dyn Shape> = Box::new(Circle::new(2.0));
    // jrust:L26
    println!("{} area={}", s.name(), s.area());
}

