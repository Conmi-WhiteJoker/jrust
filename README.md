# jrust

**Java 风格的 Rust 转译器（教学项目）。**

你写的是 Java 样子的 `.jrust` 源码，jrust 把它翻译成一份**人能读懂**的 `.rs` 文件，
再交给 `rustc` / `cargo` 编译。

它不是「用 Java 写 Rust」，也不是要替代 Rust —— 它是一条给 Java 从业者的**过渡阶梯**：
保留你熟悉的语法形状，把被 Java 隐藏起来的所有权、借用、`Result`、`Option`
摊开在生成的代码里，让你对着两份文件学。

## 快速开始

```bash
cargo build

# 转译（输出到同目录的 .rs）
./target/debug/jrust build examples/HelloWorld.jrust

# 端到端验证：转译 examples/*.jrust，再用 cargo 编译产物
./verify.sh

# 直接解释执行（不经过 rustc，断点行号就是 .jrust 行号）
./target/debug/jrust run examples/Ownership.jrust

# 命令行调试
./target/debug/jrust debug examples/Ownership.jrust

# 解释执行 vs 编译执行 输出对拍
./tests/interp_parity.sh

# move 的非法边界用例（每种都应报错并给出提示）
./tests/move_cases.sh
```

## 解释执行与调试（方案 C）

除了「转译成 `.rs`」，jrust 还内置了一个树遍历解释器，直接执行 AST：

- `jrust run <文件>`：不经过 `rustc`，立即执行并打印结果。
- `jrust debug <文件>`：命令行调试器，断点、单步、变量查看都作用在 `.jrust` 行号上。
  命令：`break <行>` / `run` / `next` / `step` / `out` / `print <名字>` / `where` / `list` / `quit`。
- `jrust dap`：以 DAP（Debug Adapter Protocol）适配器方式运行，供 IDE 连接
  （stdio 上 `Content-Length` + JSON-RPC，已实现 initialize / launch / setBreakpoints /
  configurationDone / threads / stackTrace / scopes / variables / continue / next /
  stepIn / stepOut / evaluate / disconnect）。

解释器同样保留移动语义：`String b = a;` 之后再看 `a` 会得到
「使用已移动的值」运行时错误，并提示所有权只能转移一次。

`tests/interp_parity.sh` 对每个示例比较 `jrust run` 与「转译 + cargo 跑二进制」的
stdout，要求逐字节一致——这是解释器没有跑偏的硬指标。

## 设计原则

1. **转译，不编译**。jrust 不自己生成机器码，产物是普通 Rust 源码，`rust-analyzer`、
   `cargo`、标准库全部照常可用。
2. **产物必须可读**。每段代码前带 `// jrust:L<行号>` 源映射，方便左右对照。
3. **不隐藏 Rust 的规则**。`String b = a;` 就生成移动（不自动 `clone`）；
   第二个例子会直接撞上借用检查器——这正是教学内容。
4. **砍掉 Java 的包袱**。继承、异常、反射、注解、`null` 一律不支持，
   并在报错时解释 Rust 的替代方案。
5. **零外部依赖**。`Cargo.toml` 无 `[dependencies]`，手写 lexer + 递归下降 parser，
   `cargo build` 全程不联网——学生克隆仓库后第一条命令就能跑通，不会被
   「拉不到 crates」卡住；代价是 token 表与错误恢复全靠自管。

## 保留的语法

| Java 写法 | jrust 产物 | 说明 |
|---|---|---|
| `public class Foo` | `pub struct Foo` + `impl Foo` | 数据与行为拆成两块 |
| `interface Foo` | `pub trait Foo` | |
| `implements Foo` | `impl Foo for Bar` | 方法自动分流到 trait impl / 固有 impl |
| `public Circle(double r) { this.r = r; }` | `pub fn new(r: f64) -> Self { Self { r } }` | 构造器 → 关联函数 |
| `int / long / double / float / boolean / char` | `i32 / i64 / f64 / f32 / bool / char` | |
| `i32 / i64 / i128 / isize / u8 ... u128 / usize / f32 / f64 / bool` | 同名原样输出 | Rust 原生标量类型名可直接写；解释器内部按 i64/f64 存储，i128/u128 仅支持 i64 范围内的字面量 |
| `String` | `String` | 字符串字面量自动补 `.to_string()` |
| `List<T>` | `Vec<T>`；`T` 是 interface 时 `Vec<Box<dyn T>>` | |
| `new ArrayList<>()` | `Vec::new()` | |
| `list.add(x)` / `list.size()` | `push(x)` / `len()` | |
| `for (T x : list)` | `for x in &list` | 借用而不是移动 |
| `switch` | （后续版本映射到 `match`） | MVP 未实现 |
| `System.out.println(a + "x" + b)` | `println!("{}x{}", a, b)` | 自动拆成格式串 |
| `String b = move a;` | `let b: String = a;` | jrust 扩展：显式声明搬走所有权；产物与隐式 move 相同，只多一条提示注释。合法目标：变量、本地变量的字段、方法参数；`this.字段`（`&self` 借用）与下标会报错并给替代写法 |
| `import` | 忽略（`List` 已在 prelude） | |
| `private` / `public` | 私有 / `pub` | |

## 主动砍掉的语法（会给出解释性报错）

| 砍掉 | 报错里的 Rust 替代方案 |
|---|---|
| `extends` 类继承 | trait 组合 + 字段组合，组合优于继承 |
| `try` / `catch` / `finally` / `throw` / `throws` | `Result<T, E>` + `?`；清理交给 `Drop` |
| `null` | `Option<T>`：`Some(x)` / `None` |
| `instanceof` | `enum` + `match`，或多态方法 |
| 注解 `@Xxx` | derive 宏 / 显式代码 |
| `synchronized` | `Mutex<T>` / `RwLock<T>` |
| `protected` | 只有 `pub` / 私有 |
| 反射、泛型擦除、raw type、内部类、装箱 | 没有对应物，直接不提供 |
| `++` / `--`、C 风格 `for` | `x += 1`、增强 `for` / `while` |

## MVP 的边界

- 单文件转译，无包管理、无模块系统（一个 `.jrust` 对应一个可编译的 crate 根）。
- 一个文件里恰好有一个 `main`（含 `main` 的类只允许有 `main` 方法）。
- 不支持泛型声明、`Map`、字段初始化、接口 static 方法、数组 `new`。
- 不支持字符串 `+` 拼接（只允许出现在 `println` 参数里，由 jrust 展开）。

## 目录结构

```
src/lexer.rs    手写词法分析（不引解析框架，便于阅读）
src/parser.rs   递归下降语法分析 + 解释性诊断
src/ast.rs      Java 风格的 AST（不含任何 Rust 概念）
src/codegen.rs  AST → Rust 源码（含 println 格式串展开、mut/Box 推断）
src/interp.rs   树遍历解释器（run / debug 共用，保留移动语义）
src/debug.rs    命令行调试器 + DAP 适配器
src/json.rs     零依赖 JSON（只服务 DAP）
examples/       5 个示例：HelloWorld、类与接口、多态集合、所有权移动、move 边界
tests/          move 非法边界用例 + 解释/编译输出对拍
verify/         一个空 crate，只用来编译生成的 .rs（src/bin/ 由 verify.sh 填充）
```

## 为什么不用 proc macro

宏只能看到 token，看不到「学生写错时应该怎么解释」，而且产物被编译器吞掉、不可见。
独立 CLI 的价值就在于**产物是显式的、可对照的**——这才是学习工具。
