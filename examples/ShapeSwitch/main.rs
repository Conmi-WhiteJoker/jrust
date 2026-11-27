// 由 jrust 从 examples/ShapeSwitch/main.jrust 生成。请对照 .jrust 源码阅读，不要直接改这个文件。
// 每段代码前的 `// jrust:L<行号>` 指向 .jrust 源码行。
#![allow(dead_code, unused_variables, unused_mut, unused_imports, unused_parens, non_snake_case, unused_assignments)]
use std::collections::HashMap;
use std::collections::HashSet;

// jrust:L2
#[derive(Debug, Clone)]
pub enum Shape {
    // jrust:L3
    Circle(f64),
    // jrust:L4
    Rect(f64, f64),
    // jrust:L5
    Empty,
}

// jrust:L8
#[derive(Clone)]
pub struct Printer;

impl Printer {
    // jrust:L9
    pub fn show(&self, s: &Shape) {
        // jrust:L10
        match s {
            // jrust:L11
            Shape::Circle(r) => {
                // jrust:L12
                println!("circle r={} area={}", r, 3.14159 * r * r);
            }
            // jrust:L14
            Shape::Rect(w, h) if w > h => {
                // jrust:L15
                println!("wide rect {}x{}", w, h);
            }
            // jrust:L17
            Shape::Rect(w, h) => {
                // jrust:L18
                println!("tall rect {}x{}", w, h);
            }
            // jrust:L20
            Shape::Empty => {
                // jrust:L21
                println!("empty");
            }
        }
    }
    // jrust:L26
    pub fn flag(&self, ok: bool) {
        // jrust:L27
        match ok {
            // jrust:L28
            true => {
                // jrust:L29
                println!("flag = true");
            }
            // jrust:L31
            false => {
                // jrust:L32
                println!("flag = false");
            }
        }
    }
}

// jrust:L39
fn main() {
    // jrust:L40
    let p: Printer = Printer;
    // jrust:L41
    p.show(&Shape::Circle(2.0));
    // jrust:L42
    p.show(&Shape::Rect(5.0, 2.0));
    // jrust:L43
    p.show(&Shape::Rect(2.0, 5.0));
    // jrust:L44
    p.show(&Shape::Empty);
    // jrust:L45
    p.flag(true);
    // jrust:L46
    p.flag(false);
}
