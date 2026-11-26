// 由 jrust 从 examples/MapDemo/main.jrust 生成。请对照 .jrust 源码阅读，不要直接改这个文件。
// 每段代码前的 `// jrust:L<行号>` 指向 .jrust 源码行。
#![allow(dead_code, unused_variables, unused_mut, unused_imports)]
use std::collections::HashMap;
use std::collections::HashSet;

// jrust:L8
fn main() {
    // jrust:L10
    let mut ages: HashMap<String, i32> = HashMap::new();
    // jrust:L11
    ages.insert("alice".to_string(), 30);
    // jrust:L12
    ages.insert("bob".to_string(), 25);
    // jrust:L13
    println!("size={}", ages.len());
    // jrust:L14
    println!("alice={}", ages.get("alice").cloned().unwrap());
    // jrust:L15
    println!("has carol={}", ages.contains_key("carol"));
    // jrust:L18
    ages.insert("bob".to_string(), 26);
    // jrust:L19
    println!("bob={} size={}", ages.get("bob").cloned().unwrap(), ages.len());
    // jrust:L22
    let mut prices: HashMap<String, f64> = HashMap::new();
    // jrust:L23
    println!("empty={}", prices.is_empty());
    // jrust:L24
    prices.insert("tea".to_string(), 3.5);
    // jrust:L25
    println!("tea={}", prices.get("tea").cloned().unwrap());
    // jrust:L28
    let mut names: HashMap<i32, String> = HashMap::new();
    // jrust:L29
    names.insert(1, "one".to_string());
    // jrust:L30
    println!("{} -> {}", 1, names.get(&1).cloned().unwrap());
    // jrust:L33
    let mut keys: Vec<String> = Vec::new();
    // jrust:L34
    keys.push(("alice").to_string());
    // jrust:L35
    keys.push(("bob").to_string());
    // jrust:L36
    keys.push(("carol").to_string());
    // jrust:L37
    let mut total: i32 = 0;
    // jrust:L38
    for k in &keys {
        // jrust:L39
        if ages.contains_key(k.as_str()) {
            // jrust:L40
            total = total + ages.get(k.as_str()).cloned().unwrap();
        }
    }
    // jrust:L43
    println!("total={}", total);
}
