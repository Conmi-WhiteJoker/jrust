// 由 jrust 从 examples/MapDemo/main.jrust 生成。请对照 .jrust 源码阅读，不要直接改这个文件。
// 每段代码前的 `// jrust:L<行号>` 指向 .jrust 源码行。
#![allow(dead_code, unused_variables, unused_mut, unused_imports, unused_parens, non_snake_case, unused_assignments)]
use std::collections::HashMap;
use std::collections::HashSet;

// jrust:L11
fn main() {
    // jrust:L13
    let mut ages: HashMap<String, i32> = HashMap::new();
    // jrust:L14
    ages.insert("alice".to_string(), 30);
    // jrust:L15
    ages.insert("bob".to_string(), 25);
    // jrust:L16
    println!("size={}", ages.len());
    // jrust:L19
    let alice: Option<i32> = ages.get("alice").cloned();
    // jrust:L20
    if alice.is_some() {
        // jrust:L21
        let v: i32 = alice.clone().unwrap();
        // jrust:L22
        println!("alice={}", v);
    } else {
        // jrust:L24
        println!("alice=missing");
    }
    // jrust:L26
    println!("has carol={}", ages.contains_key("carol"));
    // jrust:L29
    ages.insert("bob".to_string(), 26);
    // jrust:L30
    let bob: Option<i32> = ages.get("bob").cloned();
    // jrust:L31
    if bob.is_some() {
        // jrust:L32
        let v: i32 = bob.clone().unwrap();
        // jrust:L33
        println!("bob={} size={}", v, ages.len());
    }
    // jrust:L37
    let mut prices: HashMap<String, f64> = HashMap::new();
    // jrust:L38
    println!("empty={}", prices.is_empty());
    // jrust:L39
    prices.insert("tea".to_string(), 3.5);
    // jrust:L40
    let tea: Option<f64> = prices.get("tea").cloned();
    // jrust:L41
    if tea.is_some() {
        // jrust:L42
        let v: f64 = tea.clone().unwrap();
        // jrust:L43
        println!("tea={}", v);
    }
    // jrust:L47
    let mut names: HashMap<i32, String> = HashMap::new();
    // jrust:L48
    names.insert(1, "one".to_string());
    // jrust:L49
    let one: Option<String> = names.get(&1).cloned();
    // jrust:L50
    if one.is_some() {
        // jrust:L51
        let v: String = one.clone().unwrap();
        // jrust:L52
        println!("{} -> {}", 1, v);
    }
    // jrust:L56
    let mut keys: Vec<String> = Vec::new();
    // jrust:L57
    keys.push(("alice").to_string());
    // jrust:L58
    keys.push(("bob").to_string());
    // jrust:L59
    keys.push(("carol").to_string());
    // jrust:L60
    let mut total: i32 = 0;
    // jrust:L61
    for k in &keys {
        // jrust:L62
        let got: Option<i32> = ages.get(k.as_str()).cloned();
        // jrust:L63
        if got.is_some() {
            // jrust:L64
            let v: i32 = got.clone().unwrap();
            // jrust:L65
            total = total + v;
        }
    }
    // jrust:L68
    println!("total={}", total);
}
