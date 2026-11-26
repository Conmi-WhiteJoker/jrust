// 由 jrust 从 examples/Generics/main.jrust 生成。请对照 .jrust 源码阅读，不要直接改这个文件。
// 每段代码前的 `// jrust:L<行号>` 指向 .jrust 源码行。
#![allow(dead_code, unused_variables, unused_mut, unused_imports, unused_parens)]
use std::collections::HashMap;
use std::collections::HashSet;

// jrust:L2
#[derive(Clone)]
pub struct Box<T> {
    // jrust:L3
    value: T,
}

impl<T: Clone> Box<T> {
    // jrust:L5
    pub fn new(value: T) -> Self {
        Self { value }
    }
    // jrust:L9
    pub fn get(&self) -> T {
        // jrust:L10
        return self.value.clone();
    }
}

// jrust:L15
#[derive(Clone)]
pub struct Tag<T> {
    _marker: std::marker::PhantomData<T>,
}

// jrust:L18
#[derive(Clone)]
pub struct Util;

impl Util {
    // jrust:L19
    pub fn identity<T: Clone>(&self, x: T) -> T {
        // jrust:L20
        return x;
    }
}

// jrust:L25
fn main() {
    // jrust:L26
    let a: Box<i32> = Box::<i32>::new(42);
    // jrust:L27
    println!("{}", a.get());
    // jrust:L30
    let b: Box<String> = Box::new("hello".to_string());
    // jrust:L31
    println!("{}!", b.get());
    // jrust:L33
    let tag: Tag<i32> = Tag { _marker: std::marker::PhantomData };
    // jrust:L35
    let u: Util = Util;
    // jrust:L36
    println!("{}", u.identity(3) + u.identity(4));
    // jrust:L37
    println!("{}", u.identity("ab"));
}
