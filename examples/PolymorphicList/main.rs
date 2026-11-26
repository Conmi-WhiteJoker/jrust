// 由 jrust 从 examples/PolymorphicList/main.jrust 生成。请对照 .jrust 源码阅读，不要直接改这个文件。
// 每段代码前的 `// jrust:L<行号>` 指向 .jrust 源码行。
#![allow(dead_code, unused_variables, unused_mut, unused_imports)]
use std::collections::HashMap;
use std::collections::HashSet;

// jrust:L6
pub trait Animal {
    // jrust:L7
    fn speak(&self) -> String;
}

// jrust:L10
#[derive(Clone)]
pub struct Dog;

impl Animal for Dog {
    // jrust:L11
    fn speak(&self) -> String {
        // jrust:L12
        return "Woof".to_string();
    }
}

// jrust:L16
#[derive(Clone)]
pub struct Cat;

impl Animal for Cat {
    // jrust:L17
    fn speak(&self) -> String {
        // jrust:L18
        return "Meow".to_string();
    }
}

// jrust:L23
fn main() {
    // jrust:L24
    let mut animals: Vec<Box<dyn Animal>> = Vec::new();
    // jrust:L25
    animals.push(Box::new(Dog));
    // jrust:L26
    animals.push(Box::new(Cat));
    // jrust:L27
    for a in &animals {
        // jrust:L28
        println!("{}", a.speak());
        // jrust:L30
        println!("{}", a.speak());
    }
}
