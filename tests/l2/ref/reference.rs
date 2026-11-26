// L2 的手写参考实现：与 src/main.jrust 转译产物**行为等价**。
// 验收时两个 bin 各跑一遍，stdout 必须逐字节一致（SPEC §15 L2）。

use serde_json::Value;

struct Config {
    name: String,
    port: i32,
}

impl Config {
    fn new(name: String, port: i32) -> Self {
        Config { name, port }
    }

    fn of(name: String, port: i32) -> Self {
        Config::new(name, port)
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn port(&self) -> i32 {
        self.port
    }

    fn describe(&self) -> String {
        format!("{}:{}", self.name, self.port)
    }
}

fn main() {
    let text = "{\"name\":\"api\",\"port\":8080}";

    let v: Value = serde_json::from_str(text).unwrap();
    let name = v["name"].as_str().unwrap().to_string();
    let port = v["port"].as_i64().unwrap() as i32;

    let c = Config::of(name, port);
    println!("{}", c.describe());

    let out = serde_json::to_string(&serde_json::json!({
        "name": c.name(),
        "port": c.port()
    }))
    .unwrap();
    println!("{}", out);
}
