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
./target/debug/jrust build examples/HelloWorld/main.jrust

# 多文件工程：按 package 生成模块树（目录下的 *.jrust 一起构建）
./target/debug/jrust build tests/multifile

# 端到端验证：转译 examples/*/main.jrust，再用 cargo 编译产物
./verify.sh

# 直接解释执行（不经过 rustc，断点行号就是 .jrust 行号）
./target/debug/jrust run examples/Ownership/main.jrust

# 命令行调试
./target/debug/jrust debug examples/Ownership/main.jrust

# 解释执行 vs 编译执行 输出对拍
./tests/interp_parity.sh

# 转译 + rustc 编译校验，rustc 报错映射回 .jrust 行（不带参数时跑一致性套件）
./target/debug/jrust verify examples/HelloWorld/main.jrust

# 多文件工程：按整 crate 走 cargo build（依赖以最近的 Cargo.toml 为准，有 Cargo.lock 就复用）
./target/debug/jrust verify tests/l2/src

# 格式化 .jrust（4 空格缩进，raw 块原样保留）
./target/debug/jrust fmt examples/HelloWorld/main.jrust

# move 的非法边界用例（每种都应报错并给出提示）
./tests/move_cases.sh
```

## 解释执行与调试（方案 C）

`jrust check <文件>` 只做词法 / 语法 / 代码生成检查并打印诊断，**不写** `.rs`，适合接到编辑器保存钩子或 CI 的 lint 步骤。

### 多文件工程（`package`）

`jrust build <目录>` 会递归收集目录下的 `*.jrust`，按 `package` 生成 Rust 模块树：

- `package app.greet;` → `app/greet.rs`（package 最后一段必须等于文件名）；
- 含 `main` 的文件是 crate 根，**不能**声明 `package`；其他文件必须有 `package`；
- 每层父模块自动补 `pub mod 子模块;`：父模块有源文件就写进它的产物，否则生成 `app/mod.rs`；
- 跨模块的 `import app.greet.Greeter;` 转成 `use crate::app::greet::Greeter;`。

布局不合规（`JR3029`）会直接报错并给出改法；例子见 `tests/multifile/`。

`.jrust` 里用到的外部 crate 必须写在最近的 `Cargo.toml` 里（jrust 只读取校验、不生成也不改写），
缺失报 `JR5001`；`src/**` 工程里的 async 代码还要求声明 `tokio`。

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
4. **砍掉 Java 的包袱**。继承、`try/catch`、反射一律不支持；注解只保留 `@derive` / `@raw` / `@shared`
   与同名 Rust 属性透传；`null` 只在
   可空类型 `T?`（→ `Option<T>`）里出现，裸 `null` 赋给非可空类型会报 `JR3005`。
   异常只保留 `throws` / `throw` 这条映射到 `Result` 的窄路，并在报错时解释 Rust 的替代方案。
5. **零外部依赖**。`Cargo.toml` 无 `[dependencies]`，手写 lexer + 递归下降 parser，
   `cargo build` 全程不联网——学生克隆仓库后第一条命令就能跑通，不会被
   「拉不到 crates」卡住；代价是 token 表与错误恢复全靠自管。

## 保留的语法

下表每条语法都有可运行示例：优先指向 `examples/<Name>/`（`main.jrust` 与生成的 `main.rs` 成对出现，目录本身是独立 cargo 工程）；新特性若暂无独立示例，则指向 `tests/conformance/L1/` 的一致性用例。

| Java 写法 | jrust 产物 | 说明 | 示例 |
|---|---|---|---|
| `public class Foo` | `pub struct Foo` + `impl Foo` | 数据与行为拆成两块 | [ClassInterface](examples/ClassInterface/main.jrust) |
| `interface Foo` | `pub trait Foo` | | [ClassInterface](examples/ClassInterface/main.jrust) |
| `interface Foo extends A, B` | `pub trait Foo: A + B` | 实现 `Foo` 的类会自动为 `A`、`B` 也生成 `impl`（祖先接口的 impl 一并补齐） | [L1 用例](tests/conformance/L1/ok_iface_extends.jrust) |
| `implements Foo` | `impl Foo for Bar` | 方法自动分流到 trait impl / 固有 impl | [ClassInterface](examples/ClassInterface/main.jrust) |
| 方法接收者（自动推断） | `&self` / `&mut self` | 方法体写非 `@shared` 字段 / 调集合可变方法（`add`、`put` 等）/ 调 `&mut self` 方法 → `&mut self`，否则 `&self`；接口与实现取并集；`@shared` 写字段走 `Arc` 锁不产生 `&mut self`；`return this` 报 `JR4003` | [L1 用例](tests/conformance/L1/ok_mut_receiver.jrust) |
| 方法形参（自动推断） | `&T` / `&mut T` / `T` | 只读 → 借用；改字段 / 调 `&mut self` 方法 / 集合可变方法 → `&mut`；被 `move`、被返回、存进字段、交给 owned 形参、被 `spawn` 捕获 → owned。`String` 只读 → `&str`，`List` 只读 → `&[T]` / 可写 → `&mut Vec<T>`，`Map`/`Set` 同理，接口 → `&dyn I`；泛型参数与 Copy 标量恒按值；构造器形参仍按值 | [L1 用例](tests/conformance/L1/ok_borrow_param.jrust) |
| 形参借用覆盖（`@ref` / `@mut`） | 固定 `&T` / `&mut T` | 写在形参前，强制覆盖自动推断、不做二次推断，优先于接口并集。与所有权用法共存、`@ref` 但方法体会改它、Copy 标量、泛型参数、静态方法形参、接口与实现签名不一致 → `JR4001`；只能写在方法 / 接口方法的形参上（构造器与枚举载荷不支持） | [ok_override_ref](tests/conformance/L1/ok_override_ref.jrust) · [neg_override_conflict](tests/conformance/L1/neg_override_conflict.jrust) |
| 借用返回（自动推断） | `-> &str` / `-> &'a str` | 来源 = `return 借用形参` / `return this.f`（非 `@shared` String 字段）/ `return p.f` / `return helper(x)`（沿调用链）；Copy 标量、泛型、`T?`、接口、枚举不借，类与集合字段仍 owned + `.clone()`。唯一来源省略生命周期，多来源统一 `'a`；`&mut` 形参 / 类型不一致 / 形参已按值使用 → `JR3011`。调用点落到 owned 上下文时自动 `to_string` / `to_vec` / `clone` | [getter](tests/conformance/L1/ok_getter_ref.jrust) · [生命周期](tests/conformance/L1/ok_pick_lifetime.jrust) |
| 调用点实参 | `&x` / `&mut x` / `x` | 按被调形参三态生成：owned 变量裸传（之后不能再用，否则 `JR4001`）、字段 `.clone()`、字面量 `.to_string()`；借用形参和 `&str` 字面量原样传；`&mut` 实参自动把变量补成 `let mut` | [L1 用例](tests/conformance/L1/ok_borrow_list.jrust) |
| 多文件借用（全局分析） | 跨文件方法的 `&str` / `&self` / `-> &T` | `jrust build <目录>` 对全工程做一次签名分析，跨文件调用也能拿到形参三态与返回来源；外部 crate 的类型没有源码签名时按 `&T` 传参并输出 `warning[JR4101]`（不阻断构建，可用 `@ref`/`@mut` 覆盖） | [multifile](tests/multifile/app/greet.jrust) · [ok_ext_unknown](tests/conformance/L1/ok_ext_unknown.jrust) |
| `public Circle(double r) { this.r = r; }` | `pub fn new(r: f64) -> Self { Self { r } }` | 构造器 → 关联函数 | [ClassInterface](examples/ClassInterface/main.jrust) |
| `int / long / double / float / boolean / char` | `i32 / i64 / f64 / f32 / bool / char` | | [RustTypes](examples/RustTypes/main.jrust) |
| `i32 / i64 / i128 / isize / u8 ... u128 / usize / f32 / f64 / bool` | 同名原样输出 | Rust 原生标量类型名可直接写；解释器内部按 i64/f64 存储，i128/u128 仅支持 i64 范围内的字面量 | [RustTypes](examples/RustTypes/main.jrust) |
| `1_000_000` | `1000000` | Java 7+ 的下划线分隔符；`_` 只能夹在数字之间，否则报诊断。Rust 的 `42u8` 类型后缀不支持，类型写在声明上即可（`u8 b = 42;`） | [RustTypes](examples/RustTypes/main.jrust) |
| `String` | `String` | 字符串字面量自动补 `.to_string()` | [HelloWorld](examples/HelloWorld/main.jrust) |
| `List<T>` | `Vec<T>`；`T` 是 interface 时 `Vec<Box<dyn T>>` | | [VecImport](examples/VecImport/main.jrust) |
| `new ArrayList<>()` | `Vec::new()` | | [VecImport](examples/VecImport/main.jrust) |
| `int[] a = new int[n]` | `let mut a: Vec<i32> = vec![0; (n) as usize];` | 元素只支持标量 / `String`（默认值 `0` / `0.0` / `false` / `String::new()`）；`a.size()` / `a[i]` / 增强 for 与 `List` 一致；`new Foo[n]` 报 `JR3014` | [L1 用例](tests/conformance/L1/ok_newarray.jrust) |
| `list.add(x)` / `list.size()` | `push(x)` / `len()` | | [VecImport](examples/VecImport/main.jrust) |
| `for (T x : list)` | `for x in &list` | 借用而不是移动；可变遍历写 `for (T x : list.mutable())` → `for x in list.iter_mut()`（仅 `List<T>`/`T[]`，`Set` 报 `JR3028`） | [VecImport](examples/VecImport/main.jrust) |
| `list.stream().map(..).filter(..).collectList()` | `list.iter().cloned().map(..).filter(..).collect::<Vec<_>>()` | 白名单 `stream` / `map` / `filter` / `collectList`，其它方法报 `JR3017`；`cloned()` 保证按值处理元素；`collectList` 元素类型沿用基座集合 | [L1 用例](tests/conformance/L1/ok_iter_chain.jrust) |
| `for (int i = 0; i < n; i++)` | `for i in 0..n` | 只支持同一个 `int`/`long` 计数变量的初始化 + `<` / `<=` / `>` / `>=` + `i++` / `i--` / `i += 1` / `i -= 1`；其它形状报 `JR2003` 并提示改 `while` | [L1 用例](tests/conformance/L1/ok_cfor.jrust) |
| `Map<K, V>` / `HashMap<K, V>` | `HashMap<K, V>` | 自动补 `use std::collections::HashMap;`；`Integer`/`Long`/`Double`/`Boolean`/`Character` 按对应标量处理 | [MapDemo](examples/MapDemo/main.jrust) |
| `new HashMap<>()` | `HashMap::new()` | 无参构造 | [MapDemo](examples/MapDemo/main.jrust) |
| `map.put(k, v)` / `map.size()` / `map.containsKey(k)` | `insert(k, v)` / `len()` / `contains_key(&k)` | | [MapDemo](examples/MapDemo/main.jrust) |
| `map.get(k)` | `map.get(k).cloned().unwrap()` | 键缺失会 panic（Java 返回 `null`）；字符串键字面量直接传、变量生成 `.as_str()`，其它键加 `&`；先用 `containsKey` 判断 | [MapDemo](examples/MapDemo/main.jrust) |
| `map.getOrNull(k)` | `map.get(&k).cloned()` | 返回 `V?`（→ `Option<V>`），键缺失得到 `None`，不会 panic | [Nullable](examples/Nullable/main.jrust) |
| `Set<T>` / `HashSet<T>` | `HashSet<T>` | `new HashSet<>()` → `HashSet::new()`；`add`→`insert`、`size`→`len`、`isEmpty`→`is_empty`；`contains` / `remove` 自动取引用（`String` 元素走 `.as_str()`） | [L1 用例](tests/conformance/L1/ok_set.jrust) |
| `T? x` | `Option<T>` | `T? x = null;` → `None`；`T? x = v;` → `Some(v)`；`x == null` / `x != null` → `is_none()` / `is_some()`；对 `T?` 直接调用 `T` 的方法报 `JR3006` | [Nullable](examples/Nullable/main.jrust) |
| `T v = x;`（`x` 是 `T?`） | `let v: T = x.clone().unwrap();` | 显式解包：`None` 会 panic（等价于 Java 的 NPE）；先用 `if (x != null)` 判空 | [Nullable](examples/Nullable/main.jrust) |
| `class Box<T> { T value; ... }` | `pub struct Box<T>` + `impl<T: Clone> Box<T>` | 字段未使用的类型参数补 `PhantomData` 占位（`class Tag<T> {}`） | [Generics](examples/Generics/main.jrust) |
| `new Box<Integer>(v)` / `new Box<>(v)` | `Box::<i32>::new(v)` / `Box::new(v)` | 钻石写法由左侧声明类型推断实参；构造器形参里的 `T` 按实参替换（字符串字面量自动 `.to_string()`） | [Generics](examples/Generics/main.jrust) |
| `public <T> T id(T x)` | `pub fn id<T: Clone>(x: T) -> T` | 方法级类型参数写在方法名后；v1 统一要求 `T: Clone` | [Generics](examples/Generics/main.jrust) |
| `interface Mapper<T> { T map(T x); }` | `pub trait Mapper<T> { ... }` | 泛型接口；`class X implements Mapper<Integer>` → `impl Mapper<i32> for X`；接口类型字段/变量包成 `Box<dyn Mapper<i32>>` | [L1 用例](tests/conformance/L1/ok_generic_iface_enum.jrust) |
| 接口默认方法 / `static` 方法 | trait 默认实现 / 自由函数 `Iface_method(...)` | 带方法体即默认方法（`default` 可省）；`Iface.method()` 调用生成 `Iface_method()`（Rust 不允许 `Trait::f()`，E0790）；`static` 接口方法必须有方法体，否则 `JR3024` | [L1 用例](tests/conformance/L1/ok_iface_default.jrust) |
| `enum Maybe<T> { Some(T value), None }` | `pub enum Maybe<T> { Some(T), None }` | 泛型枚举；`Maybe.Some("hi")` 按左侧 `Maybe<String>` 替换实参 | [L1 用例](tests/conformance/L1/ok_generic_iface_enum.jrust) |
| `class Holder<T extends Named & Sizer> { ... }` | `pub struct Holder<T>` + `impl<T: Named + Sizer + Clone> Holder<T>` | 上界按 `&` 拆成 `+`；声明位置不带界，`impl` / 方法位置尾随 `Clone`（值语义需要）；`? extends` / `? super` 报 `JR3010` | [L1 用例](tests/conformance/L1/ok_generic_bounds.jrust) |
| `switch` | `match` | 仅支持 `enum` 与 `boolean`；必须穷尽（或写 `default:`）；case 块必须以 `break;` 结束，否则报 `JR3020` | [L1 用例](tests/conformance/L1/ok_enum_switch.jrust) |
| `break;` / `continue;` | `break;` / `continue;` | 只支持无标签形式；`outer:` / `break outer;` / `continue outer;` 报 `JR2004` | [负向用例](tests/conformance/L1/neg_labeled.jrust) |
| `enum Shape { Circle(double r), Empty }` | `pub enum Shape { Circle(f64), Empty }` | 带数据枚举；`Shape.Circle(2.0)` → `Shape::Circle(2.0)` | [L1 用例](tests/conformance/L1/ok_enum_switch.jrust) |
| `throws A, B` + `f()?` + `throw new A("x")` | `Result<T, JrustError>` + `?` + `return Err(...)` | 单文件模式共用程序级 `JrustError`；`?` 只能出现在 throws 方法内（`JR3018`），非 throws 方法的 `throw` 报 `JR3021` | [L1 用例](tests/conformance/L1/ok_throws.jrust) |
| `class X extends Exception` / `throw new IllegalArgumentException("…")` | `struct X` + `impl Error`；`panic!` | 自定义异常自动生成 `Display` / `Error` / `From<X> for JrustError`；`IllegalArgumentException` 视为编程错误直接 panic | [自定义异常](tests/conformance/L1/ok_custom_exception.jrust) · [IAE](tests/conformance/L1/ok_illegal_arg.jrust) |
| `(a, b) -> a + b` / `x -> x * 2` | `|a, b| a + b` / `|x| x * 2` | 闭包体只支持单个表达式；当前类型系统没有函数类型，作为实参转译、暂不可独立存放 | [L1 用例](tests/conformance/L1/ok_closure_transpile.jrust) |
| `System.out.println(a + "x" + b)` | `println!("{}x{}", a, b)` | 自动拆成格式串 | [PrintShort](examples/PrintShort/main.jrust) |
| `String s = "n=" + n;` | `let s: String = format!("n={}", n);` | 任意位置的字符串 `+` 链 → `format!`；纯字面量折叠成 `"ab".to_string()` | [L1 用例](tests/conformance/L1/ok_concat.jrust) |
| `println(x)` / `print(x)` / `println()` | `println!` / `print!` | `System.out.` 可省略；无参走 `println!()`；多参数时第一个参数当 Rust 格式串，`{}` 按序填入，字面花括号写 `{{` / `}}` | [PrintShort](examples/PrintShort/main.jrust) |
| `String b = a;` | `let b: String = a.clone();` | 普通赋值 = 按值复制，两个变量各持一份；类与枚举自动 `#[derive(Clone)]`，含 trait 对象等不可克隆字段时报 `JR4002`，改用 `move` | [Ownership](examples/Ownership/main.jrust) |
| `String b = move a;` | `let b: String = a;` | jrust 扩展：真正的移动，不 `clone`，之后 `a` 不可再用。合法目标：变量、本地变量的字段、方法参数；`this.字段`（`&self` 借用）与下标会报错并给替代写法 | [MoveEdges](examples/MoveEdges/main.jrust) |
| `panic("msg")` / `assert(cond, "msg")` | `panic!("{}", msg)` / `assert!(cond, "{}", msg)` | 内置函数；`assert` 也接受单参数；其它裸调用报「未知的函数」 | [L1 用例](tests/conformance/L1/ok_assert.jrust) |
| `Cls.staticMethod(x)` / `Iface.staticMethod(x)` | `Cls::staticMethod(x)` / `Iface_staticMethod(x)` | 类静态方法 → 关联函数；接口静态方法 → 顶层自由函数（Rust 不支持 `Trait::f()`） | [L1 用例](tests/conformance/L1/ok_iface_default.jrust) |
| `public async int f()` / `await e` | `pub async fn f(&self)` / `e.await` | `await` 只在 `async` 方法内合法，否则报 `JR3022`；解释器把 `await` 视为恒等 | [L1 用例](tests/conformance/L1/ok_async.jrust) |
| `spawn { ... }` | `tokio::spawn(async move { ... });` | 产物需要 tokio 运行时；捕获 `this`（`&self`）报 `JR3023` | [L1 用例](tests/conformance/L1/ok_spawn.jrust) |
| `public static async void main(String[] args)` | `#[tokio::main] async fn main()` | 异步入口需要 tokio 运行时（`examples/AsyncMain/Cargo.toml` 已声明） | [AsyncMain](examples/AsyncMain/main.jrust) |
| `raw { <Rust 源码> }` | 原样输出（前后带 `// jrust:L`） | 语句 / 表达式 / 类成员三处可用；块内不转译、不配平字符串与注释里的花括号；成员按顶层条目输出；解释器无法执行 raw；不能嵌套（报 `JR3027`）；表达式位置的**多行** raw 会包成块表达式 `{ … }`，体内可写 `let` 等语句 | [L1 用例](tests/conformance/L1/ok_raw.jrust) / [多行表达式](tests/conformance/L1/ok_raw_multiline/main.jrust) |
| `import java.util.List;` | 忽略（`List` / `ArrayList` 已内置映射到 `Vec`） | 只用于让 jrust 认识类型名 | [VecImport](examples/VecImport/main.jrust) |
| `import std.vec.Vec;` | `use std::vec::Vec;` | 非 `java.*` 路径原样转成 Rust 的 `use`（圆点换 `::`），去重后输出；`Vec<T>` 与 `List<T>` 同义 | [VecImport](examples/VecImport/main.jrust) |
| `/// 文档注释` | `/// 文档注释` | 挂到类 / 接口 / 枚举 / 枚举分支 / 字段 / 方法 / 构造器上；方法体内的 `///` 丢弃；`//` 与 `/* */` 普通注释丢弃 | [SharedState](examples/SharedState/main.jrust) |
| `private` / `public` | 私有 / `pub` | | [ClassInterface](examples/ClassInterface/main.jrust) |
| `public static final int N = 3;` | `pub const N: i32 = 3;` | `static final` → 模块级 `const`（`String` 常量产物是 `&str`）；`Cls.N` 引用生成 `N`；可变静态（`static` 非 `final`）报 `JR3013` | [L1 用例](tests/conformance/L1/ok_const.jrust) |
| `@derive(A, B)` / `@inline` | `#[derive(A, B)]` / `#[inline]` | 注解 → Rust 属性；`@derive` 与自动推导（`Clone` / `Debug`）合并；`@raw("…")` 原样输出；`@ref` / `@mut` 是形参注解（见上表），写在其它的位置报错 | [L1 用例](tests/conformance/L1/ok_attrs.jrust) |
| `@shared T f;` | `Arc<T>` / `Arc<Mutex<T>>` | 共享字段：只读 → `Arc<T>`，类内方法写它 → `Arc<Mutex<T>>`；读自动解引用，写先取锁再赋值（`x += e` 不会二次加锁死锁）；构造器自动包 `Arc::new(...)`；同一表达式里重复读同一 `Arc<Mutex<T>>` 字段会死锁（限制） | [SharedState](examples/SharedState/main.jrust) |
| `synchronized` 方法 / `synchronized (this.f) { ... }` | 方法体开头按字段名排序依次 `lock()` 类内 `@shared` 可变字段；块语句 `let _g = self.f.lock().unwrap();` | 已持锁时复用不重复加锁；锁不是 `@shared` 可变字段报 `JR3026` | [SharedState](examples/SharedState/main.jrust) |

## 参数借用：自动推断与手动覆盖

这一节是 jrust 最核心、也最容易和 Java 直觉冲突的地方，单独说明。

### 为什么需要它

Java 的参数是「引用的按值传递」——`f(p)` 之后 `p` 还指向同一个对象。如果直接映射成
Rust 的按值传参，每次调用都要把实参 move 走或深拷贝，写起来又累又不像 Java。
所以 jrust 按**方法体怎么用这个参数**推断形参形态：

| 方法体对该参数做了什么 | 生成的形参 | 调用点 |
|---|---|---|
| 只读（没赋值、没 `move`、没存字段） | `&T` | `f(&x)` |
| 调 `&mut self` 方法 / 改它的字段 / `add` 等集合可变方法 | `&mut T` | `f(&mut x)` |
| 被 `move` / 被返回 / 存进 `this` 字段 / 交给 owned 形参 / 被 `spawn` 捕获 | `T`（owned） | `f(x)`，之后 `x` 不可再用 |

类型上还有特化：`String` 只读 → `&str`；`List<T>` 只读 → `&[T]`、可写 → `&mut Vec<T>`；
`Map` / `Set` 同理；接口 → `&dyn I`；泛型类型参数和 Copy 标量恒按值。

### 想强制指定时

自动推断猜错、或者你就是要某个形态，就在形参前加注解：

```java
public void feed(@ref String s) { ... }   // 固定 &str
public void fill(@mut List<Integer> xs) { ... }  // 固定 &mut Vec<i32>
```

`@ref` / `@mut` 是**硬覆盖**，不做二次推断。但它兑现不了所有权语义，所以下面这些情况报
`JR4001`：参数被 `move` / 返回 / 存进字段 / 传给 owned 形参 / 被 `spawn` 捕获；`@ref` 但方法体
会改它；Copy 标量或泛型参数；静态方法形参；接口声明与实现类签名不一致（trait 与 impl 必须
逐字一致，否则 rustc 会报 `E0053`）。

### 返回值也能借出

方法体是 `return this.f;`（`f` 是非 `@shared` 的 `String`）或 `return 借用形参;` 时，
返回类型会推成 `-> &str` / `-> &T`，而不是每次深拷贝。来源唯一时省略生命周期，
多来源时统一成 `<'a>`；来源不可统一（`&mut` 形参、类型不一致）报 `JR3011`。
调用点把它当 owned 用时，自动补 `.to_string()` / `.to_vec()` / `.clone()`。

### 踩过的坑

- **`spawn` 会搬走捕获的变量**：`async move` 把外层变量整个移进闭包，外层再用就 `E0382`。
  jrust 在 spawn 前为每个自由变量生成一份 clone（`let __jr_sp0_x = x.clone();`），
  闭包内接回名字，于是外层原值仍可用 —— 与 Java lambda 捕获局部变量是副本一致。
  注意**不能**写成同名 shadowing（`let x = x.clone();`），那只是给同一块内存起第二个名字，
  `async move` 照样把外层 `x` 移走。
- **接口与实现的签名必须逐字一致**：显式注解只钳住一侧时（接口 `@ref`、实现要改），
  trait 是 `&[T]`、impl 是 `&mut Vec<T>`，rustc 直接 `E0053`。这类冲突在分析阶段就报出来，
  不会把编译错误推给你。

## 主动砍掉的语法（会给出解释性报错）

| 砍掉 | 报错里的 Rust 替代方案 |
|---|---|
| `extends` 类继承 | trait 组合 + 字段组合，组合优于继承 |
| `try` / `catch` / `finally` | Rust 没有异常：可能失败的操作返回 `Result<T, E>`，用 `throws` + `?` 传播；清理交给 `Drop` |
| `instanceof` | `enum` + `match`，或多态方法 |
| `protected` | 只有 `pub` / 私有 |
| 反射、泛型擦除、raw type、内部类、装箱 | 没有对应物，直接不提供 |
| `? extends` / `? super`（声明处变型） | Rust 没有变型：改用 trait object（`Box<dyn Trait>`）或写出具体类型，报 `JR3010` |
| `++` / `--`（C 风格 `for` 头部除外） | `x += 1` |

## MVP 的边界

- 单文件与多文件两种模式：单文件一个 `.jrust` 对应一个 crate 根；多文件用 `package a.b;` 生成模块树（`jrust build <目录>`）。没有包管理与依赖解析——外部 crate 必须在最近的 `Cargo.toml` 里声明（缺失报 `JR5001`），jrust 从不生成或改写 `Cargo.toml`。
- 一个文件里恰好有一个 `main`（含 `main` 的类只允许有 `main` 方法，以及 `static final` 常量）。
- 不支持实例字段初始化（`static final` 常量除外）、多维数组与 `new T[]{...}` 字面量初始化；泛型支持类 / 方法 / 接口 / 枚举与 `<T extends A & B>` 上界（→ `T: A + B`，`impl` 位置补 `Clone`），`? extends` / `? super` 通配符报 `JR3010`。
- `switch` 只接受 `enum` / `boolean` 条件；`int`、`String` 的 switch 会报 `JR3019` 并建议改 `if` 链（Rust 的 `match` 要求穷尽）。
- `throws` 目前生成程序级 `JrustError`，每个异常名对应一个 `String` 载荷分支；`try / catch / finally` 仍未实现。
- 闭包只能作为方法实参出现；没有函数类型声明，也没有 `List` 的高阶方法。
- `T?` 只支持声明 / 赋值 / `== null`、`!= null` 判空 / 显式解包；没有 `if let` 语法糖，直接对 `T?` 调方法或打印会报 `JR3006` 或「不能直接打印可空类型」。
- C 风格 `for` 只支持同一 `int`/`long` 计数变量的初始化 + `<` / `<=` / `>` / `>=` + `i++` / `i--` / `i += 1` / `i -= 1`；`i += 2`、多变量初始化、方向不一致等报 `JR2003` 并提示改 `while`，`++` / `--` 作为独立语句仍不支持。
- 解释器（`jrust run`）暂不支持 enum / switch / throws / 闭包 / raw，这些特性走 `jrust build` + cargo 验证。
- `raw` 块内 rustc 错误可由 `jrust verify` 映射回 `.jrust` 行列（`JR-X2`），并附「raw 块内第 k 行 → 原 .jrust 第 n 行」；`raw` 不能嵌套（`JR3027`）。
- `Map` 只支持 `put` / `get` / `getOrNull` / `size` / `containsKey` / `isEmpty`，不支持 `entrySet` 遍历与 `remove`；`get` 的返回值需要 `Clone`（标量、`String` 可用）。
- 迭代器链只支持 `stream` / `map` / `filter` / `collectList`；元素必须可 `Clone`（`Box<dyn Trait>` 元素不可），`map` 改变元素类型时 `collectList` 仍按基座类型推断。
- `@shared` 按「类内方法是否写该字段」决定 `Arc<T>` 还是 `Arc<Mutex<T>>`；不能在同一表达式里重复读同一个 `Arc<Mutex<T>>` 字段（两次加锁会死锁），把值先取到局部变量再运算。
- `synchronized` 只能锁定 `@shared` 可变字段（`synchronized (this.f)`），不支持锁定任意对象；`synchronized` 方法按字段名排序加锁，多个 `@shared` 可变字段时顺序固定以防交叉死锁。
- `raw` 块不能嵌套（内层 `raw { ... }` 报 `JR3027`）。
- `spawn` 捕获的局部变量在捕获前 clone（外层仍可用），但**不能**捕获 `this`（`&self` 不满足 `'static`，报 `JR3023`）；含 `spawn` 的用例因并发顺序不确定，不参与解释器逐字节对拍。
- 字符串 `+` 在任意位置可用，生成 `format!`：字面量折进格式串，其余段走 `{}`；纯字面量拼接折叠成 `"ab".to_string()`；`println` / `print` 单参数仍拆成更短的格式串。

## 目录结构

```
src/lexer.rs    手写词法分析（不引解析框架，便于阅读）
src/parser.rs   递归下降语法分析 + 解释性诊断
src/ast.rs      Java 风格的 AST（不含任何 Rust 概念）
src/codegen.rs  AST → Rust 源码（含 println 格式串展开、mut/Box 推断）
src/interp.rs   树遍历解释器（run / debug 共用，赋值复制 + 显式 move）
src/debug.rs    命令行调试器 + DAP 适配器
src/json.rs     零依赖 JSON（只服务 DAP）
examples/       13 个独立 cargo 工程（examples/Cargo.toml 是 workspace，共享 target/）：
                每个目录 = main.jrust + 生成的 main.rs + Cargo.toml，可单独 `cargo run -p <小写目录名>`；
                HelloWorld、ClassInterface、PolymorphicList、Ownership、MoveEdges、
                PrintShort、RustTypes、MapDemo、VecImport、Nullable、Generics、
                SharedState、AsyncMain（唯一带 tokio 依赖）
tests/          一致性用例（move 边界、解释/编译对拍、L1 语法、L2 多文件+外部 crate）+ verify 脚本
tests/conformance/L1/  单文件夹具。新用例用目录式 `<case>/{main.jrust, expected.txt}`；
                存量扁平的 `ok_*.jrust` / `neg_*.jrust` 仍被 runner 兼容（见 tests/l1_cases.sh 的 case_paths）
tests/l2/       独立 cargo 工程：多文件 + serde_json，双 bin（jrust 产物 / 手写参考）逐字节对拍
tests/spawn_capture/  独立 cargo 工程：spawn 捕获 clone 的真实编译运行（并发，不参与逐字节对拍）
```

## 为什么不用 proc macro

宏只能看到 token，看不到「学生写错时应该怎么解释」，而且产物被编译器吞掉、不可见。
独立 CLI 的价值就在于**产物是显式的、可对照的**——这才是学习工具。

## 未来

或许某一天，这个项目能独当一面，到时可能独立进行编译，这样就减少了很多转化的操作。

## 许可

MIT，见 [LICENSE](LICENSE)。随便看、随便改、随便拿去教人，保留版权声明即可。

