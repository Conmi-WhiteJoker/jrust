# SPEC 变更记录（CHANGELOG-spec）

记录 `docs/SPEC-v1.md` 每个 MINOR 版本新增 / 变更的需求号（JR-G16）。
格式：`版本 - 日期 - 需求号列表 - 一句话说明`。

## v1.0 — 初版

- 基线：JR-D8/D9/C7/C9/C10/C11/X1 等主干能力按 SPEC 首次落地。
- 说明：v1 目标是单文件、无继承、错误用统一 `JrustError` 枚举的子集。

## v1.1 — 2026-11-26

- **JR-C13**：`@shared` 共享字段 → `Arc<T>` / `Arc<Mutex<T>>`（按类内方法是否写字段决定）。
- **JR-C12**：`synchronized` 方法与 `synchronized (this.f) { ... }` 块 → 锁 `@shared` 可变字段。
- **JR-S5**：`public static async void main` → `#[tokio::main] async fn main()`。
- **JR-E6**：带标签 `outer:` / `break outer;` / `continue outer;` 报 `JR2004`。
- **JR-X3**：`raw` 禁止嵌套报 `JR3027`（覆盖率单列仍随 `jrust verify` 待做）。
- **JR-L2**：`///` 文档注释保留并挂到类 / 接口 / 枚举 / 分支 / 字段 / 方法 / 构造器。
- **JR-T6**：`Set<T>` / `HashSet<T>` → `HashSet<T>`（`add`/`contains`/`remove`/`size`/`isEmpty`）。
- **JR-E4**：`for (T x : c.mutable())` → `for x in c.iter_mut()`（`Set` 报 `JR3028`）。
- **JR-D6**：`interface A extends B, C` → `trait A: B + C`，实现类自动补齐祖先接口 impl。
- **JR-G16**：建立本文件，记录各 MINOR 版本的需求号。
- **JR-G5**（部分）：新增 `jrust check`（只诊断不落盘）；`fmt` / `verify` 仍未实现。
- 诊断码新增：`JR3026`（synchronized 锁对象非法）、`JR3027`（raw 嵌套）、`JR3028`（mutable 不适用）。

## v1.2 — 2026-11-26

- **JR-S4 / JR-M3**：多文件模式落地。`package a.b.c;` → `a/b/c.rs`；含 `main` 的文件为 crate 根（不得声明 package）；每层父模块自动补 `pub mod`（无源文件时生成 `a/mod.rs`）；`jrust build <目录>` 递归收集并按 package 布局产物。
- **JR-M2**（扩展）：多文件模式下跨模块 `import a.b.C;` → `use crate::a::b::C;`。
- 诊断码新增：`JR3029`（多文件布局错误：package 末段与文件名不符 / main 文件声明 package / 无 crate 根 / 多个 crate 根）。

## v1.3 — 2026-11-26

- **JR-M5**：`jrust build` 已支持文件 / 目录两种入参（目录递归、产物与源同构），状态转 ✅。
- **JR-M6**：明确 `jrust` 从不生成或改写 `Cargo.toml` / workspace，状态转 ✅。
- **JR-M4**：`build` / `check` / 目录构建都会向上找最近的 `Cargo.toml`，校验 `.jrust` 里的外部 crate 依赖，缺失报 `JR5001`；多文件模式下只对本地模块加 `crate::`，外部 crate 保持原路径。
- **JR-C15**：`src/**` 工程里的 async 代码要求 `Cargo.toml` 声明 tokio，缺失报 `JR5001` 并给出应添加的行。
- **JR-C6**：`throw new IllegalArgumentException(...)` → `panic!` + 提示注释（不要求 `throws`）；`class X extends Exception` → `struct X` + `Display` + `Error` + `From<X> for JrustError`；`IOException` 等标准错误仍走既有 `From`。JR-D5 相应放宽为「仅 `extends Exception` 例外」。
- **JR-C14 / JR-X2**：新增 `jrust verify <文件>`：转译后以 lib 模式交给 rustc 编译，rustc 报错按 `// jrust:L` 标记映射回 `.jrust` 行；E0277 追加 Send/Sync 中文提示，raw 块内错误附「raw 块内第 k 行 → 原 .jrust 第 n 行」。
- **JR-G5**（verify）：`jrust verify` 不带参数时依次运行 `tests/l1_cases.sh`、`tests/move_cases.sh`、`tests/interp_parity.sh`。
- **JR-G5**（fmt）：`jrust fmt` 统一 4 空格缩进、去行尾空白、折叠连续空行，`raw` 块逐字保留（含未格式化内容）；格式化后仍可正常转译编译。G5 全部子命令落地。

## v1.4 — 2026-11-26

- **JR-O7**（接收者，✅）：新增 `src/analysis.rs`，按方法体是否真的修改 `this` 状态推断接收者 —— 写非 `@shared` 字段 / 调集合可变方法（`add`/`put`/`clear` 等）/ 调本类或接口的 `&mut self` 方法 → `&mut self`，否则 `&self`。接口与实现取并集（任一实现需要 `&mut self`，trait 声明也变 `&mut self`），由 worklist 不动点求解。`synchronized` 块与 `spawn` 子块不参与（字段访问走 `Arc` 锁 / 闭包捕获）。
- **JR-O7**（`return this`）：builder 风格本版不做，直接报 `JR4003`，提示改 `this.clone()` 或静态工厂。
- 调用点可变性传播（接收者部分）：`x.inc()` 中 `inc` 为 `&mut self` 时，`let x` 自动补 `mut`，形参自动写成 `mut p`，避免把编译错误推给用户。
- 过渡开关：`JRUST_BORROW=legacy` 回退到「全 `&self`」的旧行为（F 阶段删除）。
- 诊断码新增：`JR4003`。

## v1.5 — 2026-11-26

- **JR-O4**（参数三态，✅）：`analysis.rs` 在不动点里按方法体对每个形参的用法推断 `Ref` / `MutRef` / `Value`（偏序 `Ref < MutRef < Value`，改为从 `Ref` 起步向上合并）。只读 → 借用；写字段 / 调 `&mut self` 方法 / 调集合可变方法 / 自身被重新赋值以外的改写 → `&mut`；被 `move`、被返回、存进字段、交给 owned 形参、被 `spawn` 捕获、整体重新赋值 → owned。
- **JR-O4**（类型特化）：`String` 只读 → `&str`（可变用法按值，不生成 `&mut String`）；`List`/数组只读 → `&[T]`、可写 → `&mut Vec<T>`；`Map` → `&HashMap` / `&mut HashMap`；`Set` → `&HashSet` / `&mut HashSet`；接口 → `&dyn I` / `&mut dyn I`；泛型类型参数（`T`、`List<T>`…）与 Copy 标量恒按值。构造器形参本版仍全部按值。
- **JR-O10**（调用点，✅）：按被调方法形参三态发实参 —— owned 变量裸传（登记为已移动）、字段 `.clone()`、字面量 `.to_string()`；借用加 `&`（借用形参 / `&str` 字面量原样）；`&mut` 加 `&mut x`（名字级并集补齐 `let mut`）。落地转换：借用形参的 String → `.to_string()`、List → `.to_vec()`、Map/Set/类 → `.clone()`，下标 / `get()` 取出非 Copy 元素同样补转换；有借用形参的 `for (T x : c)` 生成 `c.iter()`；`map.get` 的字符串键对 `&str` 直接传。
- **JR-O5**（部分，✅）：调用点把 owned 实参移动后又使用同一变量时报 `JR4001`；隐式推断本身按 D7「所有权优先」不报冲突。显式 `@ref`/`@mut` 的冲突校验留到 D 阶段。
- **JR-O9**（getter 借出，✅）：方法体恰为 `return this.f;` 且 `f` 是非 `@shared` 的 `String` 字段 → `-> &str`（返回处补 `&`），接口方法取「所有实现都是纯 getter」才统一借出，否则整组降级 owned；调用点落到 owned 上下文补 `.to_string()`。
- 顺带修正：`size()`/`length()` 落回 `int` / `long` 上下文时补 `as i32` / `as i64`（`len()` 是 `usize`）。
- L1 新增用例：`ok_borrow_param`、`ok_borrow_string`、`ok_borrow_list`、`ok_getter_ref`、`neg_conflict`（`JR4001`），并加产物形态断言（`&mut Vec<i32>`、`&[String]`、`-> &str` 等）。L1 79 通过 0 失败，`verify.sh` / move 12 例 / 解释一致全部通过。
- 诊断码新增：`JR4001`。

## v1.6 — 2026-11-26

- **JR-T5 / JR-O9**（借用返回来源，✅）：`RetMode` 从「getter 特例」升级为 `Owned | Borrow(Vec<Origin>)`，`Origin = SelfRef | Param(i)`。对每个 `return` 表达式求来源集合：`return x`（借用形参）、`return this.f`（非 `@shared` String 字段）、`return p.f`（形参字段）、`return helper(p)`（沿调用链映射被调方法的来源）。来源集合进独立单调不动点，接口方法取默认方法 ∪ 所有实现（任一实现 owned → 整组 owned）。
- **生命周期**：唯一来源且全签名只有一个借用输入 → 省略（`-> &str`）；否则在**来源输入**上标 `<'a>`，返回 `-> &'a T`。`ok_pick_lifetime` 断言 `fn pick<'a>(&self, x: &'a str, y: &str) -> &'a str` 与 `fn choose<'a>(&self, x: &'a str, y: &'a str) -> &'a str`；`fn tag(&self) -> &str` 仍省略。
- **JR3011**（新增）：来源不可统一时报错并降级 owned —— `&mut` 形参作来源、多个来源类型不一致、来源形参已被按值使用。`neg_borrow_field` 覆盖「借 `&mut` 形参的字段」。
- **回归修正**：`return p` 不再直接判成 owned（改由来源推断决定）；借用返回方法体内的 `return` 不再被落地转换误加 `.to_string()`；借用返回落到 owned 上下文时的转换从「String 专用」推广到 `to_string` / `to_vec` / `clone`。
- L1 84 通过 0 失败（新增 `ok_pick_lifetime`、`neg_borrow_field` 与三条产物形态断言）；`verify.sh`、move 12 例、解释一致全部通过。

## v1.7 — 2026-11-26

- **JR-O6**（多文件全局签名，✅）：`build_tree` 先收集全部 `Program`，用 `analysis::analyze_all` 合成一份视图做**一次**全局分析，再把 `Sigs` 通过 `codegen::generate_multi_with_sigs` 注入每个文件的生成；诊断仍按文件单独跑，保证行号归属正确。跨文件调用因此能拿到接收者、形参三态与返回来源，不再退化为 owned。
- **JR-O6 / JR4101**（新增，warning 级）：接收者类型已知但本工程里没有源码（外部 crate）时，签名无法判定 → 实参默认按只读借用 `&T` 生成，并输出 `warning[JR4101]`（不阻断构建），提示用 `@ref`/`@mut` 覆盖。`diag::Diag` 新增 `Level::Warning` 与 `warning_code`，渲染为 `警告: path:line - warning[码]: 消息`。
- **JR-O10**（跨文件无类型信息）：形参三态已知但形参类型不可见时按模式生成实参（`Ref` → `&x`，字面量 / 借用值原样；`MutRef` → `&mut x`）；`generate` 系列返回 `Generated { code, warnings }`，warning 与 error 分流。
- 顺带修正：多文件模式零参静态调用的产物缺 `()`（`Greeter.create` → `Greeter::create()`）。
- 用例：`tests/multifile` 增加跨文件借用方法 `Greeter.greet(String)`（生成 `pub fn greet(&self, name: &str) -> String`，调用点 `g.greet(&who)`）；L1 新增 `ok_ext_unknown`（`run_warn` 断言 `warning[JR4101]` 与 `w.write(&text)`）。L1 85 通过 0 失败，`verify.sh` / move 12 例 / 解释一致 / `jrust verify` 全通过。

## v1.8 — 2026-11-26

- **JR-O11 / JR-D9**（显式借用覆盖，✅）：形参前可写 `@ref` / `@mut`，强制覆盖 §9.2 的自动推断 —— `@ref` 固定 `&T`，`@mut` 固定 `&mut T`，**不做二次推断**，且优先于接口并集（不动点内也不允许被 `join` 抬走）。`@mut` 落在 `String` 上仍按 D2 生成 owned（jrust 从不生成 `&mut String`）。注解只允许出现在方法 / 接口方法声明的形参位置：写在类 / 字段 / 方法声明前报「只能写在方法形参上」；构造器与枚举载荷的 `parse_params` 不走形参注解路径。`@ref`/`@mut` 不接受参数，两者同时出现报错。
- **JR-O5**（冲突校验补齐，✅）：显式标注与用法矛盾时报 `JR4001` —— 与所有权用法共存（`move` / 返回 / 存进字段 / 传给 owned 形参 / 被 `spawn` 捕获）、`@ref` 但方法体会修改它、Copy 标量、泛型类型参数、静态方法形参（不进 `Sigs`）。
- **接口一致性**（新增 `JR4001`）：接口声明标 `@ref`、实现类需要可变 → trait 是 `&[T]`、impl 是 `&mut Vec<T>`，生成的 Rust 过不了 `E0053`。`analysis::check_override_consistency` 在不动点后比对 trait 与 impl 的形参三态，不一致即报错，不把编译错误推给 rustc。
- 落点：`ast::Param` 增加 `attrs` / `line`；`parser` 拆出 `parse_param_annotations` / `parse_method_params`（非形参位置仍走 `parse_annotations` 报错）；`analysis` 抽出 `scan_usage`（用法扫描与 Copy/泛型强制 Value 解耦）、`param_explicit_mode`、`check_override_conflicts`、`check_override_consistency`。
- 用例：L1 新增 `ok_override_ref`（`@mut` 把只读用法也写成 `&mut Vec<i32>`、`@ref` 锁住 `&str`、接口与实现同步）与 `neg_override_conflict`（`JR4001` 所有权冲突），并加产物形态断言（`fn touch(&self, xs: &mut Vec<i32>)`、`u.touch(&mut xs)`）。L1 90 通过 0 失败，`verify.sh` / move 12 例 / 解释一致 / `jrust verify` 全通过。

## v1.9 — 2026-11-26

- **JR-C11**（`spawn` 捕获克隆，✅）：`async move` 会把捕获的变量整个搬进闭包，外层再用就 `E0382`。新增 `spawn_free_vars`（自由变量表：块内声明的不算，`each_expr` 递归拿到 `x` / `x.f` / `x[i]` / `x.m()` 的根），在 `tokio::spawn` **之前**生成 `let __jr_sp<N>_x = x.clone();`，闭包内再 `let x = __jr_sp<N>_x;` 接回名字。关键是**不能用同名 shadowing**（`let x = x.clone();`）——那只是给同一块内存起第二个名字，`async move` 照样把外层 `x` 移走，实测确认过。Copy 标量不 clone（`async move` 直接复制）；不可 Clone 的类型报 `JR4002`；`this` 仍报 `JR3023`。新增 `Codegen::spawn_seq` 保证同一方法里多次 spawn 不撞临时名。
- **与解释器对拍的边界**：`spawn` 是真并发、输出顺序不确定，解释器是同步执行。含 `spawn` 的用例因此不参与 `tests/interp_parity.sh` 的逐字节对拍 —— 新增 `tests/spawn_capture/`（独立 cargo 工程 + tokio），在 L1 里用 `run_spawn_capture` 做真实编译运行并按行计数断言（`hello` 一次、`1` 两次）。
- 用例：L1 新增 `ok_spawn_capture`（产物形态断言 `let __jr_sp0_s = s.clone();` / `let s = __jr_sp0_s;`，且 Copy 标量 `n` 不 clone）与 `run_spawn_capture`（cargo + tokio 真实编译运行）。L1 94 通过 0 失败，`verify.sh` / move 12 例 / 解释一致 / `jrust verify` 全通过。

## v1.10 — 2026-11-26

- **修复 `@mut String` 的调用点（`E0308`）**：D2 规定 `@mut String` 落 owned，但此前只在 codegen 的 `effective_mode` 里改了签名，`Sigs` 仍是 `MutRef`，调用点于是生成 `f(&mut s)`，与 `fn f(s: String)` 冲突。现在把 D2 归一提前到分析阶段：新增 `analysis::param_sig_mode`，把 `@mut String` 钳成 `Value` 写进 `Sigs`，签名与调用点同时落 owned；调用方之后再用该变量由 `JR4001` 拦下（与 owned 语义一致）。
- 用例：`ok_override_ref` 增补 `note(@mut String)` 与 `fn note(&self, s: String)` / `u.note(note)` 两条产物断言（原来的用例没有 `@mut String`，问题被全绿掩盖）。L1 96 通过 0 失败，`verify.sh` / move 12 例 / 解释一致 / `jrust verify` 全通过。
- 文档：设计文档 §13.14 增补勘误（`@mut String` 的 `Sigs` 归一）。

## v1.11 — 2026-11-26

- **F 收尾（清理）**：删除 `JRUST_BORROW=legacy` 过渡开关（`analysis::legacy_mode` 与 `analyze` 里的早退分支），jrust 只保留借用推断这一条路径；SPEC JR-O7 同步去掉「可回退旧行为」的说明。
- **删除死代码**：`codegen::generate_multi`（Stage C 起由 `generate_multi_with_sigs` 取代）与 `expr_is_str_literal`，`cargo build` 零警告。
- 验证：L1 96 通过 0 失败，`verify.sh` / move 12 例 / 解释一致 / `jrust verify` 全通过。
