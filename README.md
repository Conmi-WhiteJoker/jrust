# jrust

**Java 风格的 Rust 转译器（教学项目）。**

你写的是 Java 样子的 `.jrust` 源码，jrust 把它翻译成一份**人能读懂**的 `.rs` 文件，
再交给 `rustc` / `cargo` 编译。

它不是「用 Java 写 Rust」，也不是要替代 Rust —— 它是一条给 Java 从业者的**过渡阶梯**：
保留你熟悉的语法形状，把被 Java 隐藏起来的所有权、借用、`Result`、`Option`
摊开在生成的代码里，让你对着两份文件学。

## 初衷

> 「这里有太多人都需要照顾了，无论是谁都行。我只是想给他们提供一个安身之所罢了。」
> —— 《十日终焉》乔家劲

那间旅馆不大，也谈不上什么宏图，只是给四处投奔来的人一个能落脚的地方。jrust 是同一件事的另一种写法。

Rust 很好，可对写了多年 Java 的人来说，所有权、借用、生命周期像一堵墙。墙后面不是没有路，只是第一次站到墙前的人，往往连门朝哪边开都看不清。

做 jrust 不是为了造一门新语言，也不是要证明 Java 能写 Rust。只是这些年在 Java 里讨生活的兄弟姐妹们，总该有条退路：写的还是熟悉的语法，生成的是能读懂的裸 Rust，一行行对照着看，墙就慢慢变成台阶。

它可能不大，也不完美——不支持的特性都明明白白写在文档里，报错时会告诉你为什么、该换成什么。它没打算骗你「Java 也能无缝写 Rust」，只想在你真的要往外走一步时，提供一个能安身的地方。

## 快速开始

```bash
cargo build

# 新建项目（main.jrust + 即时转译的 main.rs + Cargo.toml，可直接 cargo run；目录必须不存在）
./target/debug/jrust new /tmp/hello-jrust

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

解释器与转译器语义一致：普通赋值复制一份（对象、列表、Map 递归复制），源变量仍可用；
只有显式 `move` 才转移所有权，之后再看原变量会得到「使用已移动的值」运行时错误。

`tests/interp_parity.sh` 对每个示例比较 `jrust run` 与「转译 + cargo 跑二进制」的
stdout，要求逐字节一致——这是解释器没有跑偏的硬指标。

## 设计原则

1. **转译，不编译**。jrust 不自己生成机器码，产物是普通 Rust 源码，`rust-analyzer`、
   `cargo`、标准库全部照常可用。
2. **产物必须可读**。每段代码前带 `// jrust:L<行号>` 源映射，方便左右对照。
3. **不隐藏 Rust 的规则**。普通赋值按值复制（生成 `.clone()`，两个变量各自独立），
   想搬走所有权就显式写 `move`；不会静默地把两个变量绑到同一块可变数据上。
4. **砍掉 Java 的包袱**。继承、`try/catch`、反射、注解一律不支持；`null` 只在
   可空类型 `T?`（→ `Option<T>`）里出现，裸 `null` 赋给非可空类型会报 `JR3005`。
   异常只保留 `throws` / `throw` 这条映射到 `Result` 的窄路，并在报错时解释 Rust 的替代方案。
5. **零外部依赖**。`Cargo.toml` 无 `[dependencies]`，手写 lexer + 递归下降 parser，
   `cargo build` 全程不联网——学生克隆仓库后第一条命令就能跑通，不会被
   「拉不到 crates」卡住；代价是 token 表与错误恢复全靠自管。

## 保留的语法

下表每条语法都有可运行示例：`examples/` 下 `Xxx.jrust` 源码与 jrust 生成的 `Xxx.rs` 产物成对出现。

| Java 写法 | jrust 产物 | 说明 | 示例 |
|---|---|---|---|
| `public class Foo` | `pub struct Foo` + `impl Foo` | 数据与行为拆成两块 | [ClassInterface](examples/ClassInterface.jrust) |
| `interface Foo` | `pub trait Foo` | | [ClassInterface](examples/ClassInterface.jrust) |
| `implements Foo` | `impl Foo for Bar` | 方法自动分流到 trait impl / 固有 impl | [ClassInterface](examples/ClassInterface.jrust) |
| `public Circle(double r) { this.r = r; }` | `pub fn new(r: f64) -> Self { Self { r } }` | 构造器 → 关联函数 | [ClassInterface](examples/ClassInterface.jrust) |
| `int / long / double / float / boolean / char` | `i32 / i64 / f64 / f32 / bool / char` | | [RustTypes](examples/RustTypes.jrust) |
| `i32 / i64 / i128 / isize / u8 ... u128 / usize / f32 / f64 / bool` | 同名原样输出 | Rust 原生标量类型名可直接写；解释器内部按 i64/f64 存储，i128/u128 仅支持 i64 范围内的字面量 | [RustTypes](examples/RustTypes.jrust) |
| `1_000_000` | `1000000` | Java 7+ 的下划线分隔符；`_` 只能夹在数字之间，否则报诊断。Rust 的 `42u8` 类型后缀不支持，类型写在声明上即可（`u8 b = 42;`） | [RustTypes](examples/RustTypes.jrust) |
| `String` | `String` | 字符串字面量自动补 `.to_string()` | [HelloWorld](examples/HelloWorld.jrust) |
| `List<T>` | `Vec<T>`；`T` 是 interface 时 `Vec<Box<dyn T>>` | | [VecImport](examples/VecImport.jrust) |
| `new ArrayList<>()` | `Vec::new()` | | [VecImport](examples/VecImport.jrust) |
| `int[] a = new int[n]` | `let mut a: Vec<i32> = vec![0; (n) as usize];` | 元素只支持标量 / `String`（默认值 `0` / `0.0` / `false` / `String::new()`）；`a.size()` / `a[i]` / 增强 for 与 `List` 一致；`new Foo[n]` 报 `JR3014` | [L1 用例](tests/conformance/L1/ok_newarray.jrust) |
| `list.add(x)` / `list.size()` | `push(x)` / `len()` | | [VecImport](examples/VecImport.jrust) |
| `for (T x : list)` | `for x in &list` | 借用而不是移动 | [VecImport](examples/VecImport.jrust) |
| `for (int i = 0; i < n; i++)` | `for i in 0..n` | 只支持同一个 `int`/`long` 计数变量的初始化 + `<` / `<=` / `>` / `>=` + `i++` / `i--` / `i += 1` / `i -= 1`；其它形状报 `JR2003` 并提示改 `while` | [L1 用例](tests/conformance/L1/ok_cfor.jrust) |
| `Map<K, V>` / `HashMap<K, V>` | `HashMap<K, V>` | 自动补 `use std::collections::HashMap;`；`Integer`/`Long`/`Double`/`Boolean`/`Character` 按对应标量处理 | [MapDemo](examples/MapDemo.jrust) |
| `new HashMap<>()` | `HashMap::new()` | 无参构造 | [MapDemo](examples/MapDemo.jrust) |
| `map.put(k, v)` / `map.size()` / `map.containsKey(k)` | `insert(k, v)` / `len()` / `contains_key(&k)` | | [MapDemo](examples/MapDemo.jrust) |
| `map.get(k)` | `map.get(k).cloned().unwrap()` | 键缺失会 panic（Java 返回 `null`）；字符串键字面量直接传、变量生成 `.as_str()`，其它键加 `&`；先用 `containsKey` 判断 | [MapDemo](examples/MapDemo.jrust) |
| `map.getOrNull(k)` | `map.get(&k).cloned()` | 返回 `V?`（→ `Option<V>`），键缺失得到 `None`，不会 panic | [Nullable](examples/Nullable.jrust) |
| `T? x` | `Option<T>` | `T? x = null;` → `None`；`T? x = v;` → `Some(v)`；`x == null` / `x != null` → `is_none()` / `is_some()`；对 `T?` 直接调用 `T` 的方法报 `JR3006` | [Nullable](examples/Nullable.jrust) |
| `T v = x;`（`x` 是 `T?`） | `let v: T = x.clone().unwrap();` | 显式解包：`None` 会 panic（等价于 Java 的 NPE）；先用 `if (x != null)` 判空 | [Nullable](examples/Nullable.jrust) |
| `switch` | `match` | 仅支持 `enum` 与 `boolean`；必须穷尽（或写 `default:`）；case 块必须以 `break;` 结束，否则报 `JR3020` | [L1 用例](tests/conformance/L1/ok_enum_switch.jrust) |
| `enum Shape { Circle(double r), Empty }` | `pub enum Shape { Circle(f64), Empty }` | 带数据枚举；`Shape.Circle(2.0)` → `Shape::Circle(2.0)` | [L1 用例](tests/conformance/L1/ok_enum_switch.jrust) |
| `throws A, B` + `f()?` + `throw new A("x")` | `Result<T, JrustError>` + `?` + `return Err(...)` | 单文件模式共用程序级 `JrustError`；`?` 只能出现在 throws 方法内（`JR3018`），非 throws 方法的 `throw` 报 `JR3021` | [L1 用例](tests/conformance/L1/ok_throws.jrust) |
| `(a, b) -> a + b` / `x -> x * 2` | `|a, b| a + b` / `|x| x * 2` | 闭包体只支持单个表达式；当前类型系统没有函数类型，作为实参转译、暂不可独立存放 | [L1 用例](tests/conformance/L1/ok_closure_transpile.jrust) |
| `System.out.println(a + "x" + b)` | `println!("{}x{}", a, b)` | 自动拆成格式串 | [PrintShort](examples/PrintShort.jrust) |
| `println(x)` / `print(x)` / `println()` | `println!` / `print!` | `System.out.` 可省略；无参走 `println!()`；多参数时第一个参数当 Rust 格式串，`{}` 按序填入，字面花括号写 `{{` / `}}` | [PrintShort](examples/PrintShort.jrust) |
| `String b = a;` | `let b: String = a.clone();` | 普通赋值 = 按值复制，两个变量各持一份；类与枚举自动 `#[derive(Clone)]`，含 trait 对象等不可克隆字段时报 `JR4002`，改用 `move` | [Ownership](examples/Ownership.jrust) |
| `String b = move a;` | `let b: String = a;` | jrust 扩展：真正的移动，不 `clone`，之后 `a` 不可再用。合法目标：变量、本地变量的字段、方法参数；`this.字段`（`&self` 借用）与下标会报错并给替代写法 | [MoveEdges](examples/MoveEdges.jrust) |
| `import java.util.List;` | 忽略（`List` / `ArrayList` 已内置映射到 `Vec`） | 只用于让 jrust 认识类型名 | [VecImport](examples/VecImport.jrust) |
| `import std.vec.Vec;` | `use std::vec::Vec;` | 非 `java.*` 路径原样转成 Rust 的 `use`（圆点换 `::`），去重后输出；`Vec<T>` 与 `List<T>` 同义 | [VecImport](examples/VecImport.jrust) |
| `private` / `public` | 私有 / `pub` | | [ClassInterface](examples/ClassInterface.jrust) |

## 主动砍掉的语法（会给出解释性报错）

| 砍掉 | 报错里的 Rust 替代方案 |
|---|---|
| `extends` 类继承 | trait 组合 + 字段组合，组合优于继承 |
| `try` / `catch` / `finally` | Rust 没有异常：可能失败的操作返回 `Result<T, E>`，用 `throws` + `?` 传播；清理交给 `Drop` |
| `instanceof` | `enum` + `match`，或多态方法 |
| 注解 `@Xxx` | derive 宏 / 显式代码 |
| `synchronized` | `Mutex<T>` / `RwLock<T>` |
| `protected` | 只有 `pub` / 私有 |
| 反射、泛型擦除、raw type、内部类、装箱 | 没有对应物，直接不提供 |
| `++` / `--`（C 风格 `for` 头部除外） | `x += 1` |

## MVP 的边界

- 单文件转译，无包管理、无模块系统（一个 `.jrust` 对应一个可编译的 crate 根）。
- 一个文件里恰好有一个 `main`（含 `main` 的类只允许有 `main` 方法）。
- 不支持泛型声明、字段初始化、接口 static 方法、多维数组与 `new T[]{...}` 字面量初始化。
- `switch` 只接受 `enum` / `boolean` 条件；`int`、`String` 的 switch 会报 `JR3019` 并建议改 `if` 链（Rust 的 `match` 要求穷尽）。
- `throws` 目前生成程序级 `JrustError`，每个异常名对应一个 `String` 载荷分支；`try / catch / finally` 仍未实现。
- 闭包只能作为方法实参出现；没有函数类型声明，也没有 `List` 的高阶方法。
- `T?` 只支持声明 / 赋值 / `== null`、`!= null` 判空 / 显式解包；没有 `if let` 语法糖，直接对 `T?` 调方法或打印会报 `JR3006` 或「不能直接打印可空类型」。
- C 风格 `for` 只支持同一 `int`/`long` 计数变量的初始化 + `<` / `<=` / `>` / `>=` + `i++` / `i--` / `i += 1` / `i -= 1`；`i += 2`、多变量初始化、方向不一致等报 `JR2003` 并提示改 `while`，`++` / `--` 作为独立语句仍不支持。
- 解释器（`jrust run`）暂不支持 enum / switch / throws / 闭包，这些特性走 `jrust build` + cargo 验证。
- `Map` 只支持 `put` / `get` / `getOrNull` / `size` / `containsKey` / `isEmpty`，不支持 `entrySet` 遍历与 `remove`；`get` 的返回值需要 `Clone`（标量、`String` 可用）。
- 字符串 `+` 只在 `println` / `print` 的单参数位置可用（由 jrust 展开成格式串）；赋值、方法实参、返回值、下标赋值等位置报 `JR3015`，整条 `+` 链只报一次。

## 目录结构

```
src/lexer.rs    手写词法分析（不引解析框架，便于阅读）
src/parser.rs   递归下降语法分析 + 解释性诊断
src/ast.rs      Java 风格的 AST（不含任何 Rust 概念）
src/codegen.rs  AST → Rust 源码（含 println 格式串展开、mut/Box 推断）
src/interp.rs   树遍历解释器（run / debug 共用，赋值复制 + 显式 move）
src/debug.rs    命令行调试器 + DAP 适配器
src/json.rs     零依赖 JSON（只服务 DAP）
examples/       10 个示例（各含 .jrust 源码 + 生成的 .rs）：
                HelloWorld、ClassInterface、PolymorphicList、Ownership、MoveEdges、
                PrintShort、RustTypes、MapDemo、VecImport、Nullable
tests/          一致性用例（move 边界、解释/编译对拍、L1 语法）+ verify 脚本
verify/         一个空 crate，只用来编译生成的 .rs（src/bin/ 由 verify.sh 填充）
```

## 为什么不用 proc macro

宏只能看到 token，看不到「学生写错时应该怎么解释」，而且产物被编译器吞掉、不可见。
独立 CLI 的价值就在于**产物是显式的、可对照的**——这才是学习工具。

## 未来

或许某一天，这个项目能独当一面，到时可能独立进行编译，这样就减少了很多转化的操作。

## 许可

MIT，见 [LICENSE](LICENSE)。随便看、随便改、随便拿去教人，保留版权声明即可。

