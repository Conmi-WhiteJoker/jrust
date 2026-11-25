// 由 jrust 从 examples/MapDemo.jrust 生成。请对照 .jrust 源码阅读，不要直接改这个文件。
// 每段代码前的 `// jrust:L<行号>` 指向 .jrust 源码行。
#![allow(dead_code, unused_variables, unused_mut, unused_imports)]
use std::collections::HashMap;

// jrust:L6
fn main() {
    // jrust:L8
    let mut ages: HashMap<String, i32> = HashMap::new();
    // jrust:L9
    ages.insert("alice".to_string(), 30);
    // jrust:L10
    ages.insert("bob".to_string(), 25);
    // jrust:L11
    println!("size={}", ages.len());
    // jrust:L12
    println!("alice={}", ages.get(&"alice".to_string()).cloned().unwrap());
    // jrust:L13
    println!("has carol={}", ages.contains_key(&"carol".to_string()));
    // jrust:L16
    let mut names: HashMap<i32, String> = HashMap::new();
    // jrust:L17
    names.insert(1, "one".to_string());
    // jrust:L18
    println!("{} -> {}", 1, names.get(&1).cloned().unwrap());
}

