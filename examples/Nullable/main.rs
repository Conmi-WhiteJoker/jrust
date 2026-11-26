// 由 jrust 从 examples/Nullable/main.jrust 生成。请对照 .jrust 源码阅读，不要直接改这个文件。
// 每段代码前的 `// jrust:L<行号>` 指向 .jrust 源码行。
#![allow(dead_code, unused_variables, unused_mut, unused_imports, unused_parens)]
use std::collections::HashMap;
use std::collections::HashSet;

// jrust:L1
#[derive(Clone)]
pub struct Holder {
    // jrust:L2
    tag: Option<String>,
}

impl Holder {
    // jrust:L4
    pub fn new(tag: Option<String>) -> Self {
        Self { tag }
    }
    // jrust:L8
    pub fn getTag(&self) -> Option<String> {
        // jrust:L9
        return self.tag.clone();
    }
}

// jrust:L14
fn main() {
    // jrust:L15
    let a: Holder = Holder::new(Some("alpha".to_string()));
    // jrust:L16
    let b: Holder = Holder::new(None);
    // jrust:L18
    if a.getTag().is_some() {
        // jrust:L19
        // jrust 提示: 从可能为空的 `T?` 解包，None 会 panic；先确认判过空（`if (x != null)`）。
        let t: String = a.getTag().unwrap();
        // jrust:L20
        println!("{}", t);
    }
    // jrust:L22
    if b.getTag().is_none() {
        // jrust:L23
        println!("none");
    }
    // jrust:L26
    let mut m: HashMap<String, String> = HashMap::new();
    // jrust:L27
    m.insert("k".to_string(), "v".to_string());
    // jrust:L28
    let hit: Option<String> = m.get("k").cloned();
    // jrust:L29
    let miss: Option<String> = m.get("nope").cloned();
    // jrust:L30
    if hit.is_some() {
        // jrust:L31
        // jrust 提示: 从可能为空的 `T?` 解包，None 会 panic；先确认判过空（`if (x != null)`）。
        let s: String = hit.clone().unwrap();
        // jrust:L32
        println!("{}", s);
    }
    // jrust:L34
    if miss.is_none() {
        // jrust:L35
        println!("missing");
    }
}
