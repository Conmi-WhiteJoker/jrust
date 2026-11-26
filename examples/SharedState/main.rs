// 由 jrust 从 examples/SharedState/main.jrust 生成。请对照 .jrust 源码阅读，不要直接改这个文件。
// 每段代码前的 `// jrust:L<行号>` 指向 .jrust 源码行。
#![allow(dead_code, unused_variables, unused_mut, unused_imports, unused_parens)]
use std::collections::HashMap;
use std::collections::HashSet;

// jrust:L5
/// 示例 11：共享状态——`@shared` 字段与 `synchronized`。
///
/// `@shared` 只读字段生成 `Arc<T>`；被类内方法写的字段生成 `Arc<Mutex<T>>`。
/// `synchronized` 方法与 `synchronized (this.value) { ... }` 块都锁在共享字段上。
#[derive(Clone)]
pub struct Counter {
    // jrust:L7
    /// 当前计数值（方法会写 → `Arc<Mutex<i32>>`）。
    value: std::sync::Arc<std::sync::Mutex<i32>>,
    // jrust:L9
    /// 标签（只读 → `Arc<String>`）。
    label: std::sync::Arc<String>,
}

impl Counter {
    // jrust:L11
    pub fn new(label: String) -> Self {
        Self { value: std::sync::Arc::new(std::sync::Mutex::new(0)), label: std::sync::Arc::new(label) }
    }
    // jrust:L17
    /// 线程安全的累加。
    pub fn add(&self, d: i32) {
        let mut _g_value = self.value.lock().unwrap();
        // jrust:L18
        (*_g_value) = (*_g_value) + d;
    }
    // jrust:L22
    /// 同步块写法：一次持锁完成两次累加。
    pub fn addTwice(&self, d: i32) {
        // jrust:L23
        {
            let mut _g = self.value.lock().unwrap();
            // jrust:L24
            (*_g) = (*_g) + d;
            // jrust:L25
            (*_g) = (*_g) + d;
        }
    }
    // jrust:L29
    pub fn get(&self) -> i32 {
        // jrust:L30
        return (*self.value.lock().unwrap());
    }
    // jrust:L33
    pub fn name(&self) -> String {
        // jrust:L34
        return (*self.label).clone();
    }
}

// jrust:L39
fn main() {
    // jrust:L40
    let mut c: Counter = Counter::new("shared".to_string());
    // jrust:L41
    c.add(3);
    // jrust:L42
    c.addTwice(2);
    // jrust:L43
    println!("{} = {}", c.name(), c.get());
}
