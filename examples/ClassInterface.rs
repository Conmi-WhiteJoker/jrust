// 由 jrust 从 examples/ClassInterface.jrust 生成。请对照 .jrust 源码阅读，不要直接改这个文件。
// 每段代码前的 `// jrust:L<行号>` 指向 .jrust 源码行。
#![allow(dead_code, unused_variables, unused_mut, unused_imports)]
use std::collections::HashMap;

// jrust:L5
pub trait Shape {
    // jrust:L6
    fn area(&self) -> f64;
    // jrust:L7
    fn name(&self) -> String;
}

// jrust:L10
#[derive(Clone)]
pub struct Circle {
    // jrust:L11
    radius: f64,
}

impl Shape for Circle {
    // jrust:L17
    fn area(&self) -> f64 {
        // jrust:L18
        return 3.14159 * self.radius * self.radius;
    }
    // jrust:L21
    fn name(&self) -> String {
        // jrust:L22
        return "Circle".to_string();
    }
}

impl Circle {
    // jrust:L13
    pub fn new(radius: f64) -> Self {
        Self { radius }
    }
}

// jrust:L26
#[derive(Clone)]
pub struct Rectangle {
    // jrust:L27
    w: f64,
    // jrust:L28
    h: f64,
}

impl Shape for Rectangle {
    // jrust:L35
    fn area(&self) -> f64 {
        // jrust:L36
        return self.w * self.h;
    }
    // jrust:L39
    fn name(&self) -> String {
        // jrust:L40
        return "Rectangle".to_string();
    }
}

impl Rectangle {
    // jrust:L30
    pub fn new(w: f64, h: f64) -> Self {
        Self { w, h }
    }
}

// jrust:L45
fn main() {
    // jrust:L46
    let s: Box<dyn Shape> = Box::new(Circle::new(2.0));
    // jrust:L48
    println!("{} area={}", s.name(), s.area());
    // jrust:L50
    println!("{} area={}", s.name(), s.area());
    // jrust:L54
    let mut shapes: Vec<Box<dyn Shape>> = Vec::new();
    // jrust:L55
    shapes.push(Box::new(Circle::new(1.0)));
    // jrust:L56
    shapes.push(Box::new(Rectangle::new(3.0, 4.0)));
    // jrust:L58
    let mut total: f64 = 0.0;
    // jrust:L59
    for sh in &shapes {
        // jrust:L61
        print!("{}", sh.name());
        // jrust:L62
        println!(" area={}", sh.area());
        // jrust:L63
        total = total + sh.area();
    }
    // jrust:L65
    println!("total={}", total);
}

