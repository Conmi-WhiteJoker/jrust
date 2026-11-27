# jrust 语言规范 v1

> 编号：JR-SPEC-1.0　状态：草案（Draft）　日期：2026-11-25
> 适用仓库：`/root/jrust`（转译器）、`.jrust` 源文件、生成的 `.rs` 产物
> 本文只规定**外部可见的语言与契约**，不规定 lexer/parser 的内部结构。实现细节见 README 与源码。
> **v2 增量**：自 v1.46 起实现按 [SPEC-v2.md](SPEC-v2.md)（JR-SPEC-2.0）运行；本文保留为 1.0 基线，
> §13.5 起的新增契约以 v2 §2 为准。

## 0. 阅读约定

- 规范性关键词：**必须 / 必须不 / 应当 / 可以**（对应 MUST / MUST NOT / SHOULD / MAY）。
- 需求编号：`JR-<域><序号>`，域取 `S`(源文件) `L`(词法) `T`(类型) `D`(声明) `E`(表达式) `O`(所有权) `C`(并发) `M`(模块) `G`(转译契约) `X`(诊断)。
- 状态图例：`✅` v1 已实现　`🚧` v1 规划（未实现）　`❌` 永久不做（见 §13）　`❓` 待确认（见 §16）

## 1. 定位与分层目标

### 1.1 分层

| 层 | 目标 | 约束 |
|---|---|---|
| 形态层（长期不变） | 源码级转译：`.jrust` → 可读 `.rs` → rustc | 不生成机器码、不自有运行时、不做 Rust 替代品 |
| 短期目标（v1.x） | 覆盖单文件、同步、纯计算 + 基础 Result / 集合 | 产物可读性 > 语法覆盖度 |
| 终极目标（v2+） | **用 Java 风格写出任意 Rust 程序** | 见 §15 验收等级 L0–L5 |

### 1.2 硬约束

- **JR-G1** 产物必须是普通 Rust 源码，`cargo` / `rust-analyzer` / 标准库照常可用。
- **JR-G2** 产物必须可读：每条语句带 `// jrust:L<行>` 源映射（v1 行级，v1.2 起附列号）。
- **JR-G3** 转译必须是**确定性**的：同一输入两次输出逐字节相同。
- **JR-G4** 转译器不得静默改变语义；无法等价表达时必须报诊断，不得猜测。**唯一例外**是本文明确写出的映射规则（如 `+` 字符串拼接、`synchronized` 加锁）。

## 2. 术语

| 术语 | 含义 |
|---|---|
| 单元 | 一个 `.jrust` 文件 |
| 工程 | 一个含 `Cargo.toml` 的目录 |
| 产物 | `.jrust` 同级同名 `.rs` |

## 3. 源文件与编译单位

- **JR-S1** 源文件扩展名必须是 `.jrust`，UTF-8 编码，LF 或 CRLF 均可（生成统一 LF）。
- **JR-S2** 一个 `.jrust` 对应一个 `.rs`，同目录同名；`a.jrust` → `a.rs`。
- **JR-S3** 单文件模式：文件中恰有一个 `main` 类，该类只允许包含 `main` 方法（v1 保留限制）。✅
- **JR-S4** 多文件模式（✅）：`package a.b.c;` 声明模块路径，转译为 `a/b/c.jrust` → `a/b/c.rs` + 各层 `mod` 声明；`main` 所在文件作为 crate 根。布局错误（package 末段与文件名不符、main 文件声明 package、无 main 根）报 `JR3029`。
- **JR-S5** 入口：`public static void main(String[] args)` → `fn main()`；`public static async void main(String[] args)` → `#[tokio::main] async fn main()`（✅）。

## 4. 词法

- **JR-L1** 标识符：`[A-Za-z_][A-Za-z0-9_]*`；撞 Rust 关键字时由 codegen 加 `r#`（✅ v1.25，覆盖类型名 / 模块段 / 变量名 / import 路径）。`crate` / `self` / `Self` / `super` 无法用 `r#` 转义，报 `JR3030`。
- **JR-L2** 注释：`//` 行注释、`/* */` 块注释（丢弃）；`///` 文档注释 → `///`，挂到类 / 接口 / 枚举 / 枚举分支 / 字段 / 方法 / 构造器上（✅）；方法体内的 `///` 按普通注释丢弃；产物中的 `// jrust:` 前缀为保留注释，用户注释不得使用该前缀。
- **JR-L3** 整数：十进制、`0x`、`0b`；允许 Java 7+ 下划线分隔，仅可夹在数字之间（`1_000` 合法，`1_`、`1__0`、`1._5` 报 `JR1003`）。✅
- **JR-L4** 类型后缀（`42u8`）不支持，类型写在声明上。✅
- **JR-L5** 浮点：十进制与指数形式；`2.0` 显示为 `2`（与 Rust Display 一致，跨解释/编译一致）。
- **JR-L6** 字符字面量 → `char`；字符串字面量 → `&str`，在期望 `String` 的位置补 `.to_string()`。✅
- **JR-L7** `null` 只允许出现在可空类型上下文（`T?`，见 §5.2）；裸 `null` 赋给非可空类型报 `JR3005`。✅
- **JR-L8** 关键字 = Java 关键字全集（`class/interface/implements/extends/public/private/protected/static/final/void/return/if/else/while/for/new/this/true/false/null/import/package/break/continue/synchronized/try/catch/finally/throw/throws/instanceof`）
  \+ jrust 扩展：`move`（所有权移交）、`async`、`await`、`spawn`、`raw`（见 §13.2）。
- **JR-L9** 扩展关键字必须成对出现在词法表、kw_text 映射、AST、parser、codegen 五处，缺一即视为实现缺陷（这是既有踩坑，见 cairn §10）。

## 5. 类型

### 5.1 标量

| jrust 写法 | Rust | 状态 |
|---|---|---|
| `int` / `long` / `double` / `float` / `boolean` / `char` / `void` | `i32/i64/f64/f32/bool/char/()` | ✅ |
| `i32/i64/i128/isize/u8…u128/usize/f32/f64/bool` | 同名原样 | ✅（解释器内部按 i64/f64，i128/u128 仅 i64 范围字面量） |

### 5.2 可空与引用

- **JR-T1** `T?` 表示 `Option<T>`（✅）。规则：
  - `T? x = null;` → `let x: Option<T> = None;`
  - `T? x = v;` → `let x: Option<T> = Some(v);`
  - `x != null` / `x == null` → `x.is_some()` / `x.is_none()`
  - 对 `T?` 直接调用 `T` 的方法必须报 `JR3006`，并提示 `if (x != null) { T v = x; ... }`（→ `if let Some(v) = x`）。
  - 把 `T?` 赋给非可空 `T`（`T v = x;`）→ `x.clone().unwrap()`，`None` 会 panic，并在产物里给出判空提示。
  - 形参 / 返回值 / 构造器字段同样适用：`T?` 位置传非空值自动包 `Some(...)`，传 `null` 生成 `None`。
- **JR-T2** 引用语法 `&T` / `&mut T` **不在 .jrust 中暴露**（Java 没有该语法）。借用由 §9 规则自动生成；需要强制指定时用 `@ref` / `@mut` 注解覆盖推断（✅ v1.8，见 JR-O11）。
- **JR-T3** 泛型声明：`class Box<T> {...}` → `pub struct Box<T>` + `impl<T: Clone> Box<T>` ✅；`new Box<Integer>(v)` → `Box::<i32>::new(v)`，钻石 `new Box<>(v)` 由左侧声明类型推断 ✅；方法泛型 `public <T> T id(T x)` → `pub fn id<T: Clone>(x: T) -> T` ✅。字段未使用的类型参数补 `std::marker::PhantomData` 占位（`class Tag<T> {}`）；泛型接口 `interface Mapper<T>` → `pub trait Mapper<T>`（`implements Mapper<Integer>` → `impl Mapper<i32> for X`），泛型枚举 `enum Maybe<T>` → `pub enum Maybe<T>` 亦已支持 ✅。
- **JR-T4** 约束：`<T extends A & B>` → `<T: A + B>`（✅）；`impl` / 方法位置尾随 `+ Clone`（值语义补 `.clone()` 需要），即 `<T: A + B + Clone>`，声明位置不带界。Java 的 `? extends T` / `? super T`（声明处变型）Rust 无对应，必须报 `JR3010` 并提示改用 trait object 或具体类型（✅）。
- **JR-T5** 生命周期：v1 **不暴露** `'a` 语法。返回借用的来源只有 `&self` 与只读形参，按来源集合推断：唯一来源且签名里只有一个借用输入 → 省略生命周期（`-> &str`）；否则在**来源输入**上标注统一的 `'a`（`fn pick<'a>(&self, x: &'a str, y: &str) -> &'a str`）。来源无法统一（`&mut` 形参、类型不一致、形参已按值使用）→ `JR3011`，提示返回 owned 或拆方法。✅ v1.6
- **JR-T6** 集合映射见附录 A。`List<T>`/`ArrayList<T>` → `Vec<T>`；元素为接口类型时 → `Vec<Box<dyn T>>`。`Map<K,V>`/`HashMap<K,V>` → `HashMap<K,V>`。`Set<T>`/`HashSet<T>` → `HashSet<T>`（✅；`add`→`insert`、`contains`/`remove` 自动取引用、`size`→`len`、`isEmpty`→`is_empty`）。

### 5.3 类型推断

- **JR-T7** 声明处类型必须写全（不支持 `var`）；初始化表达式的类型必须与声明可赋值。
  类型名解析规则（v1.44）：类型位置的名字必须能解析到「本工程声明的 class/interface/enum」
  或「内建类型名」；否则报 `JR6001`（能找到拼写相近的已声明类型时）或 `JR6102` warning
  （纯未知，可能是外部 crate 类型）。**外部类型是显式的「不可知」状态**，不是「推断失败」——
  任何依赖它的判断都必须降级为放行 + warning（`JR4101`），不得静默猜测。
- **JR-T8** 空集合字面量（`new ArrayList<>()`）的元素类型由声明类型决定；无法确定时报 `JR3012`。
- **JR-T9** 标量加宽转换（✅ v1.39）：Java 的隐式加宽（`int` → `long` → `float` → `double`，
  等级 `int/i8–i32/u8–u32 < long/i64/isize/u64/usize < float/f32 < double/f64`）在
  **let 初始化 / 普通赋值 / 方法实参 / return** 四个落点自动补转换：
  - 变量或表达式：补 `as <目标类型>`（`long m = n;` → `let m: i64 = n as i64;`）；
  - 整数字面量落到浮点目标：改写为带小数（`double d = 1;` → `let d: f64 = 1.0;`）；
  - 整数字面量落到整数目标：**原样输出**（Rust 字面量类型由上下文推断，`let m: i64 = 5;` 合法）。
  - 同级不补、窄化（`double` → `int`）不补也不报错（留待类型检查阶段 JR-T10）。
  - 加宽只在**源类型静态可知**时发生；源类型不可知（如外部 crate 调用结果）则原样输出，由 `jrust verify` 兜底。
  - 二元运算内的标量提升（`int + double`）**不在 v1.39 范围**，见 §16-8。

## 6. 声明

- **JR-D1** `public class Foo` → `pub struct Foo` + `impl Foo`；`private` → 私有。✅
- **JR-D2** 字段必须无初始化器；有初始化器报 `JR2001` 并提示写进构造器。❌（理由见 §13.1「字段初始化器」：值与「构造器显式赋值 + 结构体字面量」冲突，v1/v2 都不做）
- **JR-D3** 构造器 `public Foo(T p) { this.f = p; }` → `pub fn new(p: T) -> Self { Self { f: p } }`；同名字段简写 `Self { f }`。✅
- **JR-D4** `interface Foo` → `pub trait Foo`；`class X implements A, B` → 方法按签名分流到 `impl A for X` / 其余进 `impl X`。✅
- **JR-D5** `extends`（类继承）**永久不支持**，报 `JR2002` 并提示「字段组合 + trait」；唯一例外是 `class X extends Exception` / `extends RuntimeException`（自定义异常，见 JR-C6）。✅
- **JR-D6** `interface A extends B, C` → `trait A: B + C`；实现子接口的类会同时为所有祖先接口生成 `impl`（✅）。类继承仍不支持。
- **JR-D7** 带数据枚举（✅）：
  ```
  enum Shape { Circle(double r), Rect(double w, double h), Empty }
  ```
  →
  ```
  pub enum Shape { Circle(f64), Rect(f64, f64), Empty }
  ```
  必须是 `class` 之外的新声明种类；Java 原生常量枚举 → `#[derive(...)]` 简化形式。
- **JR-D8** `const` / `static`（✅）：`public static final int N = 3;` → `pub const N: i32 = 3;`（`String` 常量产物为 `&str`）；`Cls.N` 引用生成 `N`。可变静态（`static` 非 `final`）报 `JR3013`（Rust 需 `static mut` + unsafe，见 §13）。
- **JR-D9** 注解 `@Xxx(args)` → `#[...]`（✅）。`@derive(A, B)` 与自动推导（`Clone` / `Debug`）合并成单条 `#[derive(...)]`；`@raw("…")` 原样输出；其余按同名属性透传（`@inline` → `#[inline]`）。`@shared` 是字段注解（JR-C13，✅）；`@ref` / `@mut` 是**形参**注解，强制覆盖 §9.2 的借用推断（✅ v1.8）；写在类 / 字段 / 方法声明前等非形参位置报错，提示只能写在形参上。
- **JR-D10** 接口 `static` 方法与默认方法：默认方法 → trait 默认实现（✅）；接口 static 方法 → 顶层自由函数 `Iface_method(...)`（✅；Rust 不允许 `Trait::f()`，见 E0790）。`default` 修饰符可省略，带方法体即为默认方法；接口 static 方法必须带方法体，否则报 `JR3024`。

## 7. 语句

- **JR-E1** `T x = e;` → `let x: T = e;`；被赋值过的局部变量推断为 `let mut`。✅
- **JR-E2** `x += e` → `x = x + e`（同理 `-=` 等）。✅
- **JR-E3** `if / else if / else`：`else if` 生成 `else if`（v1.1 起；v1 为 `else { if … }`，可编译）。✅
- **JR-E4** `while` / `for (T x : c)` → `for x in &c`（借用，不移动）；集合需要可变遍历时用 `for (T x : c.mutable())` → `for x in c.iter_mut()`，仅限 `List<T>`/`T[]`，对 `Set` 等报 `JR3028`（✅）。
- **JR-E5** C 风格 `for`：`for (int i = 0; i < n; i++)` → `for i in 0..n`（✅）。只支持同一 `int`/`long` 计数变量的初始化 + `<` / `<=` / `>` / `>=` + `i++` / `i--` / `i += 1` / `i -= 1`：`<=` / `>=` 生成 `..=`，递减生成 `(a..b).rev()`；`i += 2`、多变量初始化、步进与条件方向不一致等仍报 `JR2003` 并提示改 `while`。
- **JR-E6** `break` / `continue`：支持无标签形式；带标签（`outer:` / `break outer;` / `continue outer;`）报 `JR2004`（✅）。
- **JR-E7** `switch` → `match`（✅，仅 enum / boolean），见 §8.7。
- **JR-E8** `throw` / `throws` 映射见 §10；`try/catch/finally` 不支持（JR-C4/C5、§13.1，解析期报 `JR3031`）。

## 8. 表达式

- **JR-E9** 字面量 / 标识符 / `this` / 字段 / 方法调用 / `new` / 二元 / 一元 / 索引 / 括号。✅
- **JR-E10** 方法裸调用 `foo()` 在实例方法内生成 `self.foo()`；显式 `this.foo()` 同义。✅
- **JR-E11** `new Foo(args)` → `Foo::new(args)`；接口单变量 `Shape s = new Circle(2.0)` → `Box<dyn Shape>` + `Box::new(...)`。✅
- **JR-E12** `new T[n]` → `vec![默认值; n]`（✅）。元素类型限定标量 / `String`（默认值确定：`0` / `0.0` / `false` / `\0` / `String::new()`）；`T[]` 与 `List<T>` 同映射到 `Vec`，`size/get/for-each` 共享；`new Foo[n]` 报 `JR3014` 并提示改 `List<Foo>` + `add`；多维数组与 `new T[]{...}` 字面量初始化报诊断。
- **JR-E13** 含字符串操作数的 `+` 链 → `format!`（✅）：任意位置可用（声明、赋值、方法实参、返回值、下标赋值）；字面量段折进格式串（花括号转义为 `{{`/`}}`），其余段按 `{}` 传参；纯字面量拼接折叠成 `"ab".to_string()`；`println` / `print` 单参数仍由打印宏拆链，产物更短。
- **JR-E14** `println`/`print`/`println()` 简写；多参数时首个实参作格式串，`{}` 按序填充，字面花括号写 `{{`/`}}`；占位符与参数数不匹配报 `JR3016`。✅
- **JR-E15** 集合方法映射：`list.add` → `push`、`list.size` → `len`、`list.get(i)` → `v[i]`、`list.isEmpty` → `is_empty`；`map.put/get/size/containsKey/isEmpty` 见附录 A。`v[i]` 借出元素位置，**owned 上下文**（`String s = xs.get(0)`）按元素类型补 `.to_string()` / `.clone()`，Copy 标量不补（✅ v1.25）。
- **JR-E16** `map.get(k)` 的 v1 行为固定为 `.cloned().unwrap()`（键缺失 panic，Java 返回 null）。`map.getOrNull(k)` → `Option<V>`（✅，产物 `get(&k).cloned()`），v2 起 `get` 改为返回 `Option` 并给出迁移诊断（❓）。
- **JR-E17** 闭包（✅ 转译）：`(a, b) -> expr` → `|a, b| expr`；作为实参使用；参数可带类型标注。闭包体目前只支持单个表达式。
- **JR-E18** 迭代器链（✅）：`list.stream().map(...).filter(...).collectList()` → `iter().cloned().map().filter().collect::<Vec<_>>()`；只允许白名单方法（`stream` / `map` / `filter` / `collectList`），未知方法报 `JR3017`。`cloned()` 保证按值处理元素（否则 `filter` 里是 `&&T`、`collect` 得到 `Vec<&T>`）；`collectList()` 的元素类型沿用基座集合，`map` 改变元素类型的情况 v1 不推断。
- **JR-E19** `?` 传播（✅）：`v = f()?;` 仅允许出现在声明了 `throws` 的函数内，否则 `JR3018`。
- **JR-E20** `match`：.jrust 不新增 `match` 关键字，统一由 `switch` 映射（§8.7）。

### 8.7 switch → match 规则（✅）

- 对枚举：`case Shape.Circle(double r):` → `Shape::Circle(r) =>`
- 守卫：`case Circle(r) if r > 0:` → `Circle(r) if r > 0 =>`
- `default:` → `_ =>`
- **必须穷尽**：非枚举/非布尔/非可空类型上的 `switch` 报 `JR3019`（Rust `match` 要求穷尽，Java 不要求）。
- 有 `fallthrough`（case 块无 `break` 且非空）报 `JR3020` 并提示合并分支或显式 `|` 多模式。

## 9. 所有权与借用

### 9.1 赋值与 move

- **JR-O1** 普通赋值 / 声明是**值语义**：`T b = a;` → `let b: T = a.clone();`，`a` 保持可用，两个变量互不影响。
  - 仅当右值是变量或字段（即存在两个持有者）且 `T` 非 Copy 时补 `.clone()`；字面量、`new`、方法调用结果等临时值直接绑定，不复制。
  - `T` 为类或枚举时，codegen 自动给它加 `#[derive(Clone)]`（字段全部可 Clone 时）。✅
  - 右值不可 Clone（如含 `Box<dyn Trait>` 字段）时报 `JR4002`。✅
- **JR-O2** `T b = move a;` → `let b: T = a;`，**真正转移所有权**，不 clone；之后使用 `a` 会被 rustc 拒绝。✅
- **JR-O3** `move` 合法目标：变量、方法参数、本地变量的字段。非法：Copy 标量、字面量/临时值、方法调用结果、下标（`v[i]`）、`this.字段`；每种报错必须给替代写法（`v.remove(i)`、`this.x.clone()`）。✅
- **JR-O10** 方法实参按被调方法推断出的形参三态生成：`Value` 传值（变量裸传、字段 `.clone()`、字面量 `.to_string()`），`Ref` 加 `&`（已是引用的借用形参原样传），`MutRef` 加 `&mut`（变量需 `let mut`，由名字级并集补齐）。多文件模式（v1.7）下签名表由全工程一次分析得到，跨文件调用同样适用；形参类型不可见时按模式生成（`Ref` → `&x`）。✅ v1.5

### 9.2 参数借用推断（核心规则，✅ v1.5）

Java 的对象参数语义是「引用的按值传递」，直接映射为 Rust 的按值会强制 clone。v1.2 起采用**推断 + 可覆盖**：

| 函数体内对该参数做了什么 | 生成的形参 |
|---|---|
| 只读（未赋值、未传 `move`、未存入字段） | `&T` |
| 调用 `&mut self` 方法 / 修改其字段 | `&mut T` |
| 被 `move`、被返回、被存入 `this` 字段、被传给 `move` 形参 | `T`（调用点之后该实参视为已移动） |

- **JR-O4** 非 Copy 类型参数按上表推断；Copy 标量始终按值。集合参数：只读 → `&[T]`（Map/Set 为 `&HashMap` / `&HashSet`），可变 → `&mut Vec<T>`。`String` 参数：只读 → `&str`（不生成 `&mut String`，可变用法直接按值），按值 → `String`；接口类型按 `&dyn I` / `&mut dyn I` 传。泛型类型参数（`T`、`List<T>` 等）一律按值，避免给 `&T` 引入额外的 `Display`/`Clone` 上界。构造器形参本阶段仍全部按值（✅ v1.5）。
- **JR-O5** 推断冲突：本版采用「隐式推断、所有权优先」——同一参数既用于读取又需要所有权时直接生成 owned，不报错（D7）；只在**调用点把 owned 实参移动之后又使用同一变量**时报 `JR4001`，提示 `clone` 或改成只读形参（✅ v1.5）；显式 `@ref` / `@mut` 与用法的冲突校验 ✅ v1.8（见下条 JR-O11）。
- **JR-O11** 显式借用覆盖（✅ v1.8）：形参前写 `@ref` → 固定 `&T`，`@mut` → 固定 `&mut T`，**不做二次推断**，优先于自动推断与接口并集（不动点里也不允许被 `join` 抬走）。注解只允许写在方法 / 接口方法声明的形参位置；构造器与枚举载荷不支持。
  - **`@mut` 落在 `String` 上按 D2 归一为 owned（✅ v1.10）**：jrust 从不生成 `&mut String`，因此 `@mut String` 在**分析阶段**就把 `Sigs` 钳成 `Value`，签名是 `String`、调用点裸传。这一步必须在分析阶段做——若只在 codegen 里降级签名而 `Sigs` 留着 `MutRef`，调用点会生成 `f(&mut s)` 与 `fn f(s: String)` 冲突（`E0308`）。归一之后该形参本就是 owned，所以**存进字段 / 传给 owned 形参 / 被 `spawn` 捕获都不报 `JR4001`**（与 `@mut List<T>` 不同：后者仍推成 `&mut Vec<T>`，这些用法会报错）。
  - **报 `JR4001` 的六类情况**：① 与所有权用法共存（`move` / 返回 / 存进字段 / 传给 owned 形参 / 被 `spawn` 捕获，**`@mut String` 除外**，见上）；② `@ref` 但方法体会修改它；③ Copy 标量；④ 泛型类型参数；⑤ 静态方法形参（不进 `Sigs`）；⑥ 接口声明与实现类签名不一致（trait 与 impl 必须逐字一致，否则 rustc `E0053`）。
  - **parser 层直接报错（无诊断码）**：`@ref` / `@mut` 写在非形参位置、带括号参数、同一形参上两者同时出现。
- **JR-O6** 无法判定时（接收者类型来自外部 crate、本工程没有它的源码）默认按 `&T` 借用传参，并输出 **warning** 级诊断 `JR4101`（不阻断生成），建议用 `@ref`/`@mut` 覆盖。多文件模式下签名表对**全工程**做一次分析，跨文件类的方法能拿到真实三态，不再落入默认路径。✅ v1.7
- **JR-O7** `this` 方法：不改字段 → `&self`；改字段 / 调 `&mut self` 方法 / 调集合可变方法（`add` 等）→ `&mut self`；实现接口的方法与 trait 声明接收者取**所有实现的并集**（任一实现需要 `&mut self`，trait 也用 `&mut self`）。`@shared` 字段的写走 `Arc` 锁，不产生 `&mut self`。`return this` 本版不支持，报 `JR4003`，提示改 `this.clone()` 或静态工厂。接收者推断 ✅（v1.4；v1.11 起删除 `JRUST_BORROW=legacy` 过渡开关，不再保留旧行为）。

### 9.3 clone 与生命周期

- **JR-O8** `clone` 只在普通赋值的右值是变量 / 字段时自动插入；`move`、实参、返回值不做隐式 clone，保证行为可预测。✅ 例外是「借用落地」：借用形参 / 借用返回落到 owned 上下文时按 §9.2 的转换表补 `to_string` / `to_vec` / `clone`。
- **JR-O9** 借用返回按来源推断：`return x`（借用形参）、`return this.f`（非 `@shared` 的 `String` 字段）、`return helper(x)`（沿调用链传播）都可借出；Copy 标量 / 泛型 / `T?` / 接口 / 枚举不借。类与集合字段仍返回 owned + `.clone()`（D4 ③）。单一来源省略生命周期，多来源统一 `'a`；`&mut` 形参或类型不一致 → `JR3011`。调用点落到 owned 上下文时自动做落地转换。✅ v1.6

## 10. 错误处理

- **JR-C1** `throws A, B` 是唯一错误声明方式；函数据此返回 `Result<T, JrustError>`。单文件模式共用一个程序级错误枚举 `JrustError`，每个 `throws` 名对应一个 `String` 载荷分支。✅
- **JR-C2** 同一程序内跨类 `?` 无需转换（共用 `JrustError`）；对常见标准错误生成 `From`：`IOException` → `std::io::Error`、`NumberFormatException` → `std::num::ParseIntError`、`ParseFloatError`。✅
- **JR-C3** `throw new E("msg");` → `return Err(JrustError::E("msg".to_string()));`；`throw e;` → `return Err(e);`。无 `throws` 的函数中 `throw` 报 `JR3021`，抛出的 `E` 未在任何 `throws` 列表里也报 `JR3021`。✅
- **JR-C4** `try { A } catch (E e) { B } finally { C }` **不支持**（v1.37 定案，理由见 §13.1）：解析期报 `JR3031`，提示改用 `throws` + `?` 传播；`finally` 的清理语义交给 Rust 的 `Drop`。❌
- **JR-C5** 多 `catch` 与 `catch (Exception e)` 兜底同样不支持；错误分支由 `?` 在各调用点展开，需要区分错误类型时在 `raw` 里写 `match`。❌
- **JR-C6** 常见异常映射：`IOException` → `std::io::Error`（`From` 已生成）、`SQLException` → 由绑定 crate 的错误类型（在 `raw` 里补 `From`）、`IllegalArgumentException` → `panic!` + 提示注释；用户自定义 `class X extends Exception` → `struct X` + `impl std::error::Error` + `From<X> for JrustError`（✅）。
- **JR-C7** `panic("msg")` / `assert(cond, "msg")` → `panic!` / `assert!`（✅）。裸调用解析器已放开，但只有这四个内置名有定义；其它裸调用报「未知的函数」。
- **JR-C8** 不得把 `Result` 静默 `unwrap`；除 §8 明确规定的 `map.get` 外，任何需要解包的位置必须由用户显式写 `unwrap()` 或 `?`。

## 11. 并发与异步

- **JR-C9** `async` 作为方法/函数修饰符：`public async String fetch(String u) { ... }` → `pub async fn fetch(u: &str) -> String`（✅）。
- **JR-C10** `await expr` 表达式 → `expr.await`（✅）；只在 `async` 函数内合法，否则 `JR3022`。解释器把 `await` 视为恒等（同步执行）。
- **JR-C11** `spawn { ... }` / `spawn f(x);` → `tokio::spawn(async move { ... })`（✅）。`this`（`&self`）不满足 `'static`，捕获就报 `JR3023`；其它捕获类型（标量 / String / Vec / Map / Option）本身都是 `Send + 'static`。产物需要 tokio 运行时（见 JR-C15）。
  - **捕获语义（✅ v1.9）**：`async move` 会把捕获的变量整个搬进闭包，外层再用就会 `E0382`。因此 jrust 在 `tokio::spawn` **之前**为每个自由变量生成一份副本（`let __jr_sp0_x = x.clone();`），闭包内再把它接回来（`let x = __jr_sp0_x;`）——闭包持有副本、外层原值仍可用，与 Java lambda 捕获局部变量是副本的直觉一致。Copy 标量不 clone（`async move` 直接复制）；不可 Clone 的类型报 `JR4002`。注意不能用同名 shadowing（`let x = x.clone();`）：那只是给同一块内存起第二个名字，`async move` 照样把外层 `x` 移走。
  - **与解释器对拍的边界**：`spawn` 是真并发，输出顺序不确定；解释器是同步执行。因此含 `spawn` 的用例不参与 `tests/interp_parity.sh` 的逐字节对拍（放在 `tests/spawn_capture/` 单独做 cargo 编译 + 按行计数断言）。
- **JR-C12** `synchronized` 方法 → `&self` + 方法体开头按字段名排序依次锁住类内所有 `Arc<Mutex<T>>` 的 `@shared` 字段（无则输出提示注释）；`synchronized (this.f) { ... }` → `let _g = self.f.lock().unwrap();`（已持锁则复用，不重复加锁）；锁对象不是 `@shared` 可变字段时报 JR3026（✅）。
- **JR-C13** 共享状态：`@shared T f;` → `Arc<T>`；可写共享 → `Arc<Mutex<T>>`，由类内方法是否写该字段（或对该字段调用会修改自身的方法）决定（✅）。写入时先取一次锁再赋值，避免 `x += e` 二次加锁死锁；构造器里自动包 `Arc::new(...)` / `Arc::new(Mutex::new(...))`。
- **JR-C14** `Send`/`Sync` 不做语法暴露，违反时把 rustc 的 E0277 映射回 `.jrust` 行列并给中文提示（✅，`jrust verify`）。
- **JR-C15** `async` 工程必须在 `Cargo.toml` 声明对应运行时依赖；缺失时报 `JR5001` 并给出应添加的行（✅，按 `src/**` 工程布局校验 tokio）。

## 12. 模块与工程

- **JR-M1** `import java.util.List;` 等 `java.*` 只做类型名映射，不生成 `use`。✅
- **JR-M2** 非 `java.*` 的 `import a.b.C;` → `use a::b::C;`（圆点换 `::`，去重）。✅
- **JR-M3** `package a.b;` → 目录 `a/b/`，并生成 `mod` 关联（✅）。
- **JR-M4** `Cargo.toml` 是依赖的唯一事实来源：jrust 只读取并校验，不生成、不改写；`.jrust` 中用到的 crate 必须在最近的 `Cargo.toml` 存在，否则 `JR5001`（✅）。
- **JR-M5** 工程目录约定：`src/**/*.jrust` ↔ `src/**/*.rs` 一一对应；`jrust build` 支持文件与目录两种入参（✅）。
- **JR-M6** workspace 多 crate（✅）：`jrust` 不生成 workspace 结构，`Cargo.toml` 的 `[workspace]` 由用户维护；jrust 从不写入或改写任何 `Cargo.toml`。

## 13. 互操作边界

### 13.1 永久不做（❌）

| 项 | 理由 / 处理 |
|---|---|
| 宏**定义**（`macro_rules!` / proc macro 编写） | 与「可读产物 + 可解释诊断」冲突；需要时用 `raw` 内联 |
| `unsafe` 块的语义建模 | 不做检查与建模；需要时用 `raw` 原样内联并保留在产物中 |
| `no_std` / 裸机 / `build.rs` | v1–v2 范围外 |
| 类继承、反射、注解处理器、泛型擦除模拟 | 没有对应语言机制，报错并给 Rust 方案 |
| 字段初始化器（`T f = v;`） | 与「构造器显式赋值 + 结构体字面量」冲突：初始化器会被构造器覆盖、缺省值来源不清；报 `JR2001`，值写进构造器（v1.36 定案） |
| `try / catch / finally` | 定案不支持（v1.37）：错误处理走 `throws` + `?`，资源清理交给 `Drop`；解析期报 `JR3031` 并给替代方案 |

### 13.2 raw 内联（✅）

- **JR-X1** `raw { <Rust 源码> }` 作为语句/表达式/成员时原样输出，不参与转译；必须保留前后 `// jrust:L` 标记。✅
  - 语句 / 表达式：`raw { … }` 作为一个 token，块内逐字保留（字符串、字符、注释里的花括号不参与配平）。
  - 成员：类内 `raw { … }` 按顶层条目输出（如 `fn`、`struct`、`const`），位于该类的 `impl` 之后。
  - 表达式位置生成 `(/* jrust:L行 */<原文>)`，不改变求值语义。（v1.16 起；旧形态 `({ /* … */ <原文> })` 的括号内是块表达式，`rustfmt` 必定拆成多行。）
  - 语句位置：单行 `raw` 补当前缩进；多行 `raw` 逐字保留、不加缩进（否则会改动多行字符串字面量的内容）。
- **JR-X2** `raw` 内的 rustc 错误必须映射回 `.jrust` 中 `raw` 块的起始行，并在诊断中附原文行号（✅，`jrust verify`：报出 raw 起始行 + 「raw 块内第 k 行 → 原 .jrust 第 n 行」）。
- **JR-X3** `raw` 不得嵌套（✅，报 JR3027）；`raw` 内容不计入 jrust 诊断覆盖率、在 `jrust verify` 报告中单列（✅ v1.36：单文件与目录模式都输出 `raw: N 块 / M 行（原样内联，不计入 jrust 诊断覆盖率）`，由 `l1_cases.sh` 的 `run_verify_raw` 断言）。

## 13.5 类型检查（✅ v1.41–v1.44）

`jrust check` 与 `jrust build` 都会跑类型检查；错误阻断生成，warning 只提示（JR-G6）。
设计背景见 cairn 设计文档 §14。

### 13.5.1 名称解析

- **JR-T10** 类型位置的名字必须能解析：本工程声明的 class / interface / enum、泛型参数、
  内建类型名，四者之一。否则：
  - 能找到拼写相近的已声明类型（Levenshtein ≤ 2）→ **error `JR6001`**，提示给出建议
    （「未知类型 `Intger`（是不是 `Integer`？）」）；
  - 纯未知、找不到相近名 → **warning `JR6102`**（可能是外部 crate 类型）。
- **JR-T11** `var` 不支持（JR-T7 的检查面）：`var x = 1;` 报 `JR6001` 并提示写全类型。
  `var` 与拼错的类型名走同一条规则——它在 parser 里就是一个普通的未知 `Named`。
- **JR-T12** 方法调用：接收者类型能在本工程解析到、且该类型（含实现的接口、`extends` 链）
  确实没有该方法 → **error `JR6004`**，能找到相近方法名时给建议。
  - 外部 crate 类型 → 放行（`JR4101` warning，见 JR-O6）；
  - 泛型参数 → 放行（上界方法集不做检查）；
  - `raw` 块内 → 不检查（JR-X3）。
- **JR-T13** 条件表达式必须是 `boolean`（`if` / `while`）→ 否则 **error `JR6009`**。
  Java 与 Rust 在这一点上一致，不会拒绝任何合法 Java 写法。

### 13.5.2 类型兼容（`assignable`）

- **JR-T14** 赋值 / 返回的类型必须兼容，否则 **`JR6007`**（赋值）/ **`JR6008`**（返回）。
  兼容规则按序判定：
  1. 完全相同（`bool` 的 `Type::Boolean` 与 `RustPrim("bool")` 视为相同）；
  2. `T?` 相关一律放行（判空逻辑由 JR-T1 负责）；
  3. 数值标量：同级或加宽允许，**窄化报错**；
  4. `char` → 数值允许；
  5. 类 → 接口（按 `implements` / `extends` 链）、接口 → 父接口允许；
  6. `Named("Box")` 与 `Generic("Box", args)` 同名视为兼容（目标侧常省略实参）；
  7. 泛型参数、外部 crate 类型放行；
  8. 其余不允许。
- **JR-T15** 集合 / 数组不做元素级检查：`List<String> ys = xs.stream().map(...).collectList();`
  合法，但 `collectList` 按基座类型推断（JR-E18 的已知局限）。检查边界停在该推断能力之内。

### 13.5.3 标量提升

- **JR-T9**（见 §5.3）赋值 / 实参 / `return` 四个落点的加宽转换。
- **JR-T16** 二元运算的提升：算术（`+ - * / %`）与关系（`< <= > >=`）运算，
  两侧标量类型不同时按 Java 规则把窄侧补 `as`（`n + 1.5` → `n as f64 + 1.5`）。
  **同级运算不提升**——`int / int` 保持整数除法（`7 / 2 == 3`，Java 语义）。
  `&&` / `||` 两侧必须是 `bool`，不参与提升。

### 13.5.4 内建类型方法

- **JR-E21** String 方法映射（只做语义等价换名，JR-G4）：

  | Java | Rust | 备注 |
  |---|---|---|
  | `length()` | `chars().count()` | **语义修正**：Java 数字符、Rust `len()` 数字节 |
  | `isEmpty()` / `equals(x)` / `toString()` | `is_empty()` / `eq(x)` / `to_string()` | 等价 |
  | `startsWith` / `endsWith` / `contains` | `starts_with` / `ends_with` / `contains` | 等价 |
  | `toUpperCase` / `toLowerCase` | `to_uppercase` / `to_lowercase` | 同为 Unicode 感知 |
  | `trim()` | `trim()` | Rust 去 Unicode 空白、Java 去 <= U+0020 |

  **不映射**（语义有实质差异，报 `JR6004` 并建议 `raw`）：`substring` / `charAt`
  （UTF-16 码元 vs 字节）、`split` / `replace`（正则 vs 字面量）、`indexOf`
  （返回 `int` vs `Option<usize>`）。
- **JR-T17** 内建类型方法白名单：String / List / Map / Set 各有名单，名单外报 `JR6004`。
  白名单必须与 codegen 的映射表、`infer_type` 的返回类型表保持一致。

## 14. 转译器契约

- **JR-G5** CLI 子命令：`new`、`build`、`run`、`debug`、`dap`、`check`（只诊断不落盘，✅）、`verify`（转译 + rustc 编译校验，错误映射回 `.jrust`；不带参数时跑一致性套件；**带目录入参时按整 crate 走 `cargo build`**：临时目录里白名单重建最小 manifest（只保留 `[package]` name/version/edition 与依赖段）并复用原 `Cargo.lock`，子进程统一 `--color never`；build 脚本 / 本地 path 依赖 / workspace 依赖继承不支持，直接给诊断，✅ v1.17）、`fmt`（统一 4 空格缩进 / 去行尾空白 / 折叠空行，`raw` 块逐字保留，✅）。
- **JR-G6** 退出码：`0` 成功；`1` 存在 error 级诊断；`2` IO/参数/环境错误。warning 不影响退出码。
- **JR-G7** 诊断格式（stdout）：

  ```
  <path>:<line>:<col>: error[JR3006]: 可空类型不能直接调用方法
  提示: 改为 if (x != null) { T v = x; ... }
  ```

  v1 只保证到行（列固定 0）；v1.2 起要求列号精确。v1.37 起渲染固定为 `path:line:0: error[码]: 消息`。

  该形态是**全部错误输出**的统一格式（v1.38 起）：编译期诊断（`check` / `build`）、
  `verify` 转发的 rustc 报错（`path:line:0: error[E0599]: …`）、解释器的运行时错误
  （`path:line:0: error: 消息`，无诊断码故不带方括号）三者一致，提示行统一以 `提示: ` 开头、不再缩进。
- **JR-G8** 诊断必须**一次报多条**（不在首个错误处停止），且每条 error 都必须带 `提示:` 行。
- **JR-G9** 产物以稳定、可对照为先：**不要求整份通过 `rustfmt --check`**（长表达式、结构体字面量等保持单行）。硬约束是不得因转译形态本身导致 rustc 报错，且在 `-D warnings` 下可编译（生成头的 `allow` 覆盖 `dead_code, unused_variables, unused_mut, unused_imports, unused_parens, non_snake_case, unused_assignments` —— 后两项是 Java 命名与「先声明后赋值」的生成形态噪声，v1.25）；不得输出 `todo!` / `unimplemented!`。
- **JR-G10** rustc 版本基线：`stable` ≥ 1.85（edition 2021）；CI 至少覆盖基线版与最新 stable。（✅ v1.37：`.github/workflows/ci.yml` matrix `1.85.0` + `stable`，stable 跑九步全量验收）
- **JR-G11** 规范版本与实现版本分离：`jrust --version` 同时输出 `jrust 0.x.y (spec 1.0)`。（✅ v1.37）

## 15. 一致性等级与验收

每一级都是**可执行验收**，不以语法表条数计分；`jrust verify` 按级运行。

| 等级 | 内容 | 验收标准 |
|---|---|---|
| L0 ✅ | 现有 15 个示例（含 move 边界、共享状态、异步入口、enum+switch、throws+`?`）+ 解释/编译对拍 | `./verify.sh` 与 4 个测试脚本全绿 |
| L1 ✅ | 单文件 CLI：`Result/?`、`switch→match`、带数据 enum、闭包、`T?`/`null` + `getOrNull`、C 风格 `for`、`new T[n]`、字符串 `+` → `format!`、泛型上界、标量加宽、块作用域 | `tests/conformance/L1` 正负用例 + `l1_cases.sh` 内的多文件 / 依赖 / fmt / spawn / L2 / verify 目录用例，v1.44 起共 **167 项全绿**（36 条负向用例断言行号；`check` 全量覆盖 36 正 + 43 负，另计 2 项）；诊断码 JR2003/JR3005/JR3006/JR3010/JR3014/JR3018–JR3021、JR3030、JR3031、JR6001、JR6004、JR6007–JR6009、JR-E21、JR6102 可复现 |
| L2 ✅ | 多文件工程 + 一个外部 crate（`serde_json`）。外部 crate 的**函数签名**不可知，实际调用走 `raw` 内联；jrust 负责工程结构（`package`→`mod`、`import`→`use`、依赖校验）与本地类间的借用推断。**工程内跨文件**的类型信息（构造器形参、返回类型、enum、接口、Clone 判定）由工程级类型表提供（v1.25） | `cargo build` 通过且行为与手写版本一致（`tests/l2/` 与 `tests/l2_cross/` 双 bin 逐字节对拍，`run_l2` / `run_l2_cross`）；`jrust verify <目录>` 自身由 `run_verify_tree` / `run_verify_tree_bad` / `run_verify_tree_manifest` / `run_verify_tree_lib` 覆盖。**边界**：verify 的临时工程用白名单重建 manifest，不支持 build 脚本 / 本地 path 依赖 / workspace 依赖继承（直接给诊断）；依赖版本以原 `Cargo.lock` 为准（沿目录向上找，workspace 成员的根 lockfile 也复用，v1.25）；没有 `main` 的目录按 lib crate 校验（v1.22）；`[[bin]]` 指向待验目录内 jrust 产物的按原名保留，其余 bin（手写 `.rs` / 目录外路径）跳过；`verify.sh` 为九步全量验收（cargo test / 例程构建 / l1_cases / move_cases / 解释对拍 / L3 服务 / L4 双实现对拍，v1.35）；跳过搬不走的 bin 会显式提示 |
| L3 ✅ | async 小服务：`axum` + 共享状态 + 优雅退出 | `tests/l3/`（jrust `@shared` 计数 + axum `/health`、`/hit` + SIGINT 优雅退出）由 `tests/l3_smoke.sh` 验收：端点 smoke（`/hit` 1→2）+ `ab -n 3000 -c 32` 实测 ≈11k QPS、**0 失败** + SIGINT 退出码 0 且打印 `shutdown clean`；`verify.sh` 第 8 步（v1.34） |
| L4 ✅ | async 中型服务全量转译：多模块、外部 crate、共享状态、后台任务 | `tests/l4/`（`app.gate` + `app.store` 多模块、axum 路由、`@shared` 计数、`spawn` 后台心跳）+ 手写参考实现 `ref/reference.rs` 由 `tests/l4_cases.sh` 验收：7 条功能用例状态码与 body **逐字节对拍**（含 JR-G13 的错密码 401、未知设备 404、键缺失 panic→500），3 轮稳定性基线 + `hits` 计数一致 + 后台 `ticks` 持续增长；`ab -n 2000 -c 20 /health` 两侧 0 失败、调试构建实测 ≈10k QPS（单跑峰值 ≈12.6k，generated ≥ 0.7×reference）；SIGINT 两侧退出码 0 且打印 `shutdown clean`；`verify.sh` 第 9 步（v1.35） |
| L5 ✅ | 规范附录 A 中所有「规划」项转为「已实现」 | 附录 A 已无非 ❌ 项（全部 ✅）；`jrust verify` 的 raw 统计报告（JR-X3）由 `run_verify_raw` 覆盖；除 §13.1 的 6 项永久不做外无遗留边界（v1.36） |

- **JR-G12** 一致性套件组织：`tests/conformance/<级别>/<用例>/{input.jrust, expect.rs, expect.stderr}`；`verify` 逐字节比对产物与诊断。
- **JR-G13** L4 的等价性判据必须包含**失败路径**（错密码 deny、未知设备 deny、键缺失 panic 行为），不只看正常路径。

## 16. 待确认决策（❓）

以下各项 v1 已给出默认取值，标注「需确认」；确认前按默认执行。

| # | 问题 | v1 默认 | 备选 |
|---|---|---|---|
| 1 | `async` 语法 | 修饰符位 `public async T f()` + `await e` | 隐式 await（Java 21 虚拟线程风格，无需 await 关键字） |
| 2 | 错误类型生成粒度 | 每函数一个 `<fn>Error` 枚举 | 每类一个共享错误枚举 |
| 3 | 可空语法 | `T?` + `null` 仅可赋给 `T?` | 不提供 `null`，强制 `Option` 构造语法 |
| 4 | 参数借用 | §9.2 自动推断 + `@ref/@mut` 覆盖（✅ v1.8） | 全部默认 `&T`，需要所有权时一律写 `move` |
| 5 | `map.get` 语义 | v1 保持 `.cloned().unwrap()`，新增 `getOrNull` | v2 起 `get` 直接返回 `Option`（破坏性） |
| 6 | 集合参数形态 | `List<T>` 只读参数 → `&[T]` | 统一 `&Vec<T>`（不动脑但与 Java 直觉接近） |
| 7 | 生命周期暴露 | 不暴露，必要时 `@lifetime("'a")` | 允许在 `.jrust` 写 `'a`（更 Rust、不 Java） |
| 8 | 二元运算的标量提升 | ✅ v1.42 已实现：算术 / 关系运算按 Java 规则把窄侧加宽（`n + 1.5` → `n as f64 + 1.5`）；`&&` / `\|\|` 不参与 | — |

## 17. 版本与兼容

- **JR-G14** 规范版本 `MAJOR.MINOR`：MINOR 只增加能力与诊断；MAJOR 允许改变产物形态或既有映射语义。
- **JR-G15** 破坏性映射（如 §16-5 的 `map.get`）必须提供一个 MINOR 版本的**双轨期**：旧行为保留 + 新写法可用 + 弃用警告 `JR9xxx`。（尚未实现：v1.37 决定随 V2 双轨期落地，当前没有已弃用 API；机制草案见 cairn 设计文档 §12.43 第 5 条）
- **JR-G16** 每个 MINOR 版本必须在 `docs/CHANGELOG-spec.md` 记录新增/变更需求号（✅，已建 v1.0 / v1.1 记录）。

## 附录 A：Java → jrust → Rust 映射总表

状态：✅ 已实现　🚧 规划　❌ 不做　｜　需求列对应 §3–§14 的条款号；`（§n）` 表示由该章节的类型表直接规定，无单独条款。

| 需求 | jrust 写法 | Rust 产物 | 状态 |
|---|---|---|---|
| JR-D1 | `public class Foo` | `pub struct Foo` + `impl Foo` | ✅ |
| JR-D1 | `private` / `public` 成员 | 私有 / `pub` | ✅ |
| JR-D4 | `interface Foo` | `pub trait Foo` | ✅ |
| JR-D4 | `implements A, B` | `impl A for X` / `impl B for X` | ✅ |
| JR-D3 | `public X(T p) { this.f = p; }` | `pub fn new(p: T) -> Self { Self { f: p } }` | ✅ |
| （§6） | `int/long/double/float/boolean/char/void` | `i32/i64/f64/f32/bool/char/()` | ✅ |
| JR-T9 | `long m = n;`（`int` → 更宽标量） | `let m: i64 = n as i64;`（赋值 / 实参 / return 三处；字面量 → `1.0`） | ✅ |
| （§6） | `i32…u128/isize/usize` | 同名原样输出 | ✅ |
| JR-L3 | `1_000_000` | `1000000` | ✅ |
| JR-L6 | `String` / 字符串字面量 | `String`（期望 String 处补 `.to_string()`） | ✅ |
| JR-E21 | `s.length()` / `isEmpty()` / `trim()` / `toUpperCase()` 等 | `s.chars().count()` / `is_empty()` / `trim()` / `to_uppercase()`；`substring` / `charAt` / `split` / `replace` / `indexOf` 不映射（语义不同，报 `JR6004`） | ✅ |
| JR-T10 | 类型位置写未声明的名字 | 报 `JR6001`（拼错）或 `JR6102` warning（纯未知，可能是外部类型） | ✅ |
| JR-T12 | 调用不存在的方法 | 报 `JR6004` + 拼写建议；内置类型列支持清单 + `raw` 逃生口 | ✅ |
| JR-T13 | `if (n)`（n 非 bool） | 报 `JR6009` | ✅ |
| JR-T14 | `int n = "s";` / `int f() { return "s"; }` | 报 `JR6007` / `JR6008` | ✅ |
| JR-T16 | `n + 1.5`（int + double） | `n as f64 + 1.5`；`int / int` 保持整数除法 | ✅ |
| JR-T6 | `List<T>` / `ArrayList<T>` | `Vec<T>`（元素为接口 → `Vec<Box<dyn T>>`） | ✅ |
| JR-T8 | `new ArrayList<>()` | `Vec::new()` | ✅ |
| JR-E15 | `list.add/size/get/isEmpty` | `push/len/v[i]/is_empty` | ✅ |
| JR-E4 | `for (T x : v)` | `for x in &v` | ✅ |
| JR-E5 | `for (int i = 0; i < n; i++)` | `for i in 0..n`（`<=` → `..=`；递减 → `.rev()`） | ✅ |
| JR-T6 | `Map<K,V>` / `HashMap<K,V>` | `HashMap<K,V>` + 自动 `use` | ✅ |
| JR-T8 | `new HashMap<>()` | `HashMap::new()` | ✅ |
| JR-E15 / JR-E16 | `map.put/size/containsKey/isEmpty` | `insert/len/contains_key/is_empty` | ✅ |
| JR-E16 | `map.get(k)` | `get(&k).cloned().unwrap()` | ✅ |
| JR-E16 | `map.getOrNull(k)` | `get(&k).cloned()` → `Option<V>` | ✅ |
| JR-E14 | `System.out.println(a + "x" + b)` | `println!("{}x{}", a, b)` | ✅ |
| JR-E14 | `println(x)` / `print(x)` / `println()` | `println!` / `print!` / `println!()` | ✅ |
| JR-E13 | `String s = "n=" + n;` | `format!("n={}", n)`；纯字面量 → `"ab".to_string()` | ✅ |
| JR-M1 | `import java.util.List;` | 忽略（只做类型名映射） | ✅ |
| JR-M2 | `import std.vec.Vec;` | `use std::vec::Vec;` | ✅ |
| JR-E1 | `T x = e;` | `let x: T = e;`（被赋值过 → `let mut`） | ✅ |
| JR-E2 | `i += 1` | `i = i + 1`（推断 `let mut`） | ✅ |
| JR-E3 | `if / else if / else` | `if / else if / else` | ✅ |
| JR-E6 | `break` / `continue` | 同名；带标签报 `JR2004` | ✅ |
| JR-E9 | 字面量 / 标识符 / `this` / 字段 / 调用 / `new` / 索引 / 括号 | 同名表达式 | ✅ |
| JR-E10 | 方法裸调用 `foo()` | `self.foo()`（实例方法内） | ✅ |
| JR-E11 | `new Foo(args)` | `Foo::new(args)`；接口单变量 → `Box<dyn T>` | ✅ |
| JR-O1 | `String b = a;` | `let b: String = a.clone();`（值语义） | ✅ |
| JR-O2 / JR-O3 | `String b = move a;` | `let b: String = a;`（转移所有权） | ✅ |
| JR-T1 / JR-L7 | `T? x = null;` / `T? x = v;` | `let x: Option<T> = None;` / `Some(v)` | ✅ |
| JR-T1 | `x == null` / `x != null` | `x.is_none()` / `x.is_some()` | ✅ |
| JR-T1 | `T v = x;`（`x: T?`） | `x.clone().unwrap()` + 判空提示 | ✅ |
| JR-D7 | `enum E { A(int x), B }` | `pub enum E { A(i32), B }` | ✅ |
| JR-E7 | `switch`（enum / boolean） | `match`（穷尽性检查） | ✅ |
| JR-C1 / JR-C3 / JR-E19 | `throws` / `throw` / `?` | `Result<T, JrustError>` + `?` | ✅ |
| JR-E17 | `(a) -> a * 2` | `\|a\| a * 2` | ✅ |
| JR-E19 | `f()?` | `f()?`（仅 `throws` 函数内） | ✅ |
| JR-E12 | `new T[n]` | `vec![0; n]`（标量 / `String`；对象类型报 `JR3014`） | ✅ |
| JR-T3 | `class Box<T>` / `public <T> T id(T x)` | `struct Box<T>` + `impl<T: Clone> Box<T>` / `fn id<T: Clone>(x: T) -> T` | ✅ |
| JR-T4 | `<T extends A & B>` | `<T: A + B>`（`impl` 位置补 `Clone`） | ✅ |
| JR-D8 | `public static final int N = 3;` | `pub const N: i32 = 3;`（可变静态报 `JR3013`） | ✅ |
| JR-D9 | `@derive(...)` / `@Xxx` | `#[derive(...)]` / `#[Xxx]`（`@raw("…")` 原样输出） | ✅ |
| JR-D10 | 接口默认方法 / static 方法 | trait 默认实现 / 自由函数 `Iface_method` | ✅ |
| JR-C7 | `panic("msg")` / `assert(cond, "msg")` | `panic!` / `assert!` | ✅ |
| JR-C9 / JR-C10 | `async` / `await` | `async fn` / `.await`（非 async 内 `await` → `JR3022`） | ✅ |
| JR-C11 | `spawn { }` | `tokio::spawn(async move { })`（捕获 `this` → `JR3023`；局部变量捕获前 clone，外层仍可用） | ✅ |
| JR-C12 | `synchronized` | `Mutex` / `RwLock` 加锁 | ✅ |
| JR-C13 | `@shared T f;` | `Arc<T>`（或 `Arc<Mutex<T>>`） | ✅ |
| JR-E18 | `list.stream().map(...).filter(...)` | `iter().cloned().map().filter()` | ✅ |
| JR-M3 / JR-S4 | `package a.b;` | 目录模块 + `mod` | ✅ |
| JR-X1 | `raw { … }` | 原样内联（成员 / 语句 / 表达式，前后带 `// jrust:L`） | ✅ |
| JR-D5 | `extends`（类） | —（报 `JR2002`，提示字段组合 + trait；`extends Exception` 除外） | ❌ |
| （§10） | `try/catch/finally` | 无对应；用 `throws` + `?` | ❌ |
| （§13.1） | 宏定义 / unsafe 语义 / no_std / build.rs | — | ❌ |

> 未出现在本表的条款不是「一条写法 → 一条产物」的映射：JR-L1/L5/L8/L9（标识符、浮点、关键字）、JR-T2/T5/T7（借用暴露、生命周期、声明类型）、JR-O4–O9（参数借用推断与 `&self`）、JR-C2/C4/C5/C6/C8（跨类错误转换、`try/catch` 分工、`Result` 解包纪律）、JR-M4/M5/M6（工程与 workspace）、JR-X2/X3（`raw` 诊断映射）、JR-G1–JR-G16（转译器契约）按对应章节执行，产物形态由这些规则共同决定。

## 附录 B：最小示例（v1 目标形态）

```java
// 1) Result + ? + switch→match（L1 验收）
public class Reader {
    public void run() throws IOException {
        String s = read("a.txt")?;
        println(s);
    }
}

// 2) 带数据 enum + match（L1）
enum Shape { Circle(double r), Rect(double w, double h) }

public class Area {
    public static double of(Shape s) {
        switch (s) {
            case Circle(double r): return 3.14159 * r * r;
            case Rect(double w, double h): return w * h;
        }
    }
}
```

## 附录 C：典型规模场景映射（示意，非规范）

| 通用构造 | Java 风格写法（目标） | 依赖规范条款 |
|---|---|---|
| `#[tokio::main] async fn main()` | `public static async void main(String[] args) throws Exception` | JR-S5 / JR-C9 |
| 框架 handler：提取状态/请求体并返回响应 | `@handler public async Resp h(@State State s, @Body Req r)` | JR-D9 / JR-C9（框架绑定 ❓） |
| 共享配置 + 读写锁热替换 | `@shared Config cfg;` | JR-C13 |
| 异步缓存客户端 `client.get_json(&key).await` | `cache.getJson(key)?` | JR-E15 / JR-C10 |
| `HashMap<String, i64>` 缓存 + TTL | `Map<String, Long> cache = new HashMap<>();` | 附录 A |
| `?` 传播 + 统一错误类型 | `throws Exception` + `f()?` | JR-C1 / JR-C4 |
| `tokio::spawn(worker::run(...))` | `spawn workerRun(...);` | JR-C11 |
| `match x { Some(v) => …, None => … }` | `if (v != null) { … } else { … }` | JR-T1 |

> 真实服务中密集的 `&str` / `&State` / 生命周期省略，由 §9.2 的参数借用推断承接；无法推断处（跨外部 crate 签名）依赖 JR-O6 的 warning + `@ref/@mut` 显式覆盖。
