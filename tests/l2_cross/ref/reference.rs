// L2 跨文件矩阵的手写参考实现：与 jrust 产物**行为等价**。
// 验收时两个 bin 各跑一遍，stdout 必须逐字节一致（SPEC §15 L2）。

#[derive(Clone)]
enum Color {
    Red,
    Green,
    Blue,
}

trait Shape {
    fn label(&self) -> String;
}

#[derive(Clone)]
struct Point {
    name: String,
    x: i32,
    y: i32,
}

impl Point {
    fn new(name: String, x: i32, y: i32) -> Self {
        Point { name, x, y }
    }

    fn origin() -> Self {
        Point::new("o".to_string(), 0, 0)
    }

    fn echo(s: String) -> String {
        s
    }

    fn rename(&mut self, name: String) {
        self.name = name;
    }

    fn name(&self) -> String {
        self.name.clone()
    }

    fn sum(&self) -> i32 {
        self.x + self.y
    }

    fn nickname(&self) -> Option<String> {
        if self.x == 0 {
            return None;
        }
        Some(self.name.clone())
    }

    fn tone(&self) -> Color {
        if self.x > 0 {
            Color::Green
        } else {
            Color::Red
        }
    }
}

impl Shape for Point {
    fn label(&self) -> String {
        format!("point:{}", self.name)
    }
}

fn main() {
    let mut p = Point::new("api".to_string(), 3, 4);
    println!("{}:{}", p.name(), p.sum());

    p.rename("svc".to_string());
    println!("{}", p.name());

    println!("{}", Point::echo("hi".to_string()));

    let o = Point::origin();
    println!("{}", o.sum());

    match o.nickname() {
        None => println!("none"),
        Some(v) => println!("{}", v),
    }
    if let Some(v) = p.nickname() {
        println!("{}", v);
    }

    match p.tone() {
        Color::Red => println!("red"),
        Color::Green => println!("green"),
        Color::Blue => println!("blue"),
    }

    let s: Box<dyn Shape> = Box::new(Point::new("boxed".to_string(), 1, 1));
    println!("{}!", s.label());
}
