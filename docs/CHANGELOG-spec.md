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

## v1.12 — 2026-11-26

- **修正 `JR3023` 诊断行号**：`Codegen` 的 `saw_this: bool` 改为 `this_line: Option<usize>`，`Expr::This` 首次出现时记录 `cur_line`；`spawn` 生成结束后用该行报错，不再指向块内最后一条语句。例：`spawn { println(this.x); println("done"); }` 现在报 `this` 所在行。
- 用例：`neg_spawn_this` 增加块尾语句以暴露行号偏差；`tests/l1_cases.sh` 的 `run_neg` 支持可选第 4 参数断言行号。
- 验证：L1 96 通过 0 失败，`verify.sh` / move 12 例 / 解释一致 / `jrust verify` 全通过。

## v1.13 — 2026-11-26

- **`Expr::This` 携带源码行号**（`This(usize)`，parser 取 token 行）：`spawn` 的 `JR3023` 改用它定位，多行语句不再退化成语句起始行。例：`println(` 换行后再写 `this.x`，现在报 `this.x` 所在行。
- 用例：`neg_spawn_this` 改为多行实参形态，断言行号为 `this.x` 行。
- 同步修正：`find_return_this` 的 `Expr::This` 模式改为 `This(_)`，避免 `JR4003` 因行号字段失配而漏报（由 `neg_return_this` 回归捕获）。
- 验证：L1 96 通过 0 失败，`verify.sh` / move 12 例 / 解释一致 / `jrust verify` 全通过。

## v1.14 — 2026-11-26

- **补齐 JR4001 / parser 注解错误的 L1 覆盖**：此前 `JR4001` 的六类触发只有「存进字段」一类有用例（`neg_override_conflict`），其余五类与 parser 层的三条注解错误只有实现、没有回归保护。新增 8 个用例：
  - `neg_override_ref_mutates`（`@ref` 但方法体会修改它）
  - `neg_override_scalar`（Copy 标量）
  - `neg_override_generic`（泛型类型参数）
  - `neg_override_static`（静态方法形参）
  - `neg_override_iface`（接口声明 `@ref` + 实现需要可变 → trait/impl 签名不一致，原本会漏到 rustc `E0053`）
  - `neg_override_misplaced` / `neg_override_args` / `neg_override_both`（parser 层：位置、括号参数、两者互斥）
- 新增 `tests/l1_cases.sh` 的 `run_neg_plain`：parser 层错误没有诊断码，按「错误信息片段 + 提示片段」断言，不套 `error[码]` 格式。
- **变异验证**：逐条把对应检查改坏（`is_copy_scalar` 短路、注释掉 `check_override_consistency`、放开 `@ref`/`@mut` 互斥判断），确认相应用例会变红，避免写出「永远通过」的假用例；随后完整恢复源码。
- **SPEC JR-O11 措辞修正**：明确 `@mut String` 在分析阶段就归一成 owned，因此存字段 / 传给 owned 形参 / 被 `spawn` 捕获**都不报** `JR4001`（与 `@mut List<T>` 不同）；六类触发条件单列，并补 parser 层三类直接报错。
- L1 104 通过 0 失败；`verify.sh` / move 12 例 / 解释一致 / `jrust verify` 全通过。

## v1.15 — 2026-11-26

- **修复跨 package 同名类导致的死循环**：两个同名类共享 `(类型名, 方法名)` 签名键时，`recv` 的 `Ref ↔ Mut` 非单调回写会让 `analyze` 永不收敛（`jrust build <目录>` 直接挂起）。新增 `Recv::join`，接收者与形参的合并一律只升级、不回退。
- **补齐 6 个手工缺口用例**（L1 104 → 114）：
  1. `spawn` 捕获 `this` 的正向替代路径（字段先复制到局部变量）——`ok_spawn_this_copy`；
  2. `raw` 块内自由变量不参与捕获克隆（已知限制，断言无 `__jr_sp0_s`）——`ok_spawn_raw` + `run_build_lacks`；
  3. 外部 crate 构造器实参按 owned 生成——`tests/deps_ok/ctor.jrust` + `run_deps_ctor`；
  4. 跨 package 同名类（含防挂起的 `timeout` 断言与 rustc 编译运行）——`tests/multifile_same` + `run_multifile_same`；
  5. `@ref` 不适用于构造器形参 / 枚举载荷 / 返回值位置——`neg_ref_ctor` / `neg_ref_enum` / `neg_ref_return`；
  6. 闭包按引用捕获 `this` 字段与局部变量，外层仍可用——`ok_closure_capture`（rustc 编译运行）。
- 工具：`run_neg_plain` 的第 3 参数改为可选（部分解析错误没有「提示:」行）。
- 验证：L1 114 通过 0 失败，`verify.sh` / move 12 例 / 解释一致 / `jrust verify` 全通过。

## v1.16 — 2026-11-26

- **L2 里程碑落地（SPEC §15）**：多文件工程 + 一个外部 crate（`serde_json`），验收标准「`cargo build` 通过且行为与手写版本一致」首次**可执行**。
  - 新增 `tests/l2/` 夹具：`src/**/*.jrust`（`package app` + crate 根）+ `ref/reference.rs` 手写等价实现，同一 `Cargo.toml` 下两个 bin（`generated` / `reference`），stdout 逐字节对拍。
  - `tests/l1_cases.sh` 新增 `run_l2`（转译 → 产物形态三条断言 → `cargo build` → 与 `expected.txt` 对拍 → 与手写实现对拍）与 `run_l2_missing_dep`（删掉 `serde_json` 依赖后必须报 `JR5001`）。
- **新增 `jrust verify <目录>`**（JR-G5 扩展）：单文件模式用 `--crate-type lib --emit=metadata` 就够，多文件工程不行 —— `mod` 树要求文件按真实路径排布、外部 crate 要求依赖可解析。做法是把 `plan_tree` 的产物写进临时目录、抄一份最近的 `Cargo.toml`（去掉 `[[bin]]`/`[lib]` 等 target 段）再用 `cargo build` 编整 crate，报错按产物文本映射回 `.jrust` 行。
- **重构**：`build_tree` 拆成 `plan_tree`（解析 + 全局分析 + 生成，不落盘，返回 `TreePlan`）与 `build_tree`（只负责写盘），供 `verify_tree` 复用。
- **raw 产物的两处 JR-G9 违规修复**（此前 13 个示例全部过不了 `rustfmt --check`）：
  - 产物尾部多余空行（`}\n\n`）→ `into_result` 统一削成单个 `\n`；
  - 表达式位置 `({ /* jrust:L */ x })` → `(/* jrust:L */x)`：前者的括号内是块表达式，rustfmt 必定拆成多行。
  - 顺带：单行 `raw` 语句补当前缩进（多行 raw 仍逐字保留，避免改动多行字符串内容）。
- **已知残留**：`examples/` 里仍有 4 个产物过不了 `rustfmt --check`（`Generics` / `MapDemo` / `SharedState` / `VecImport`），原因是单行超 100 列需拆行、`use` 顺序需重排 —— 属于「生成器不做行长优化」的既有缺口，未在本轮处理。
- **L2 的边界**（写进 SPEC §15）：外部 crate 的**函数签名** jrust 无法得知（没有源码可分析），因此实际调用走 `raw` 原样内联；jrust 负责工程结构（`package`→`mod`、`import`→`use`、`Cargo.toml` 依赖校验）与本地类之间的借用推断。
- 验证：L1 **116 通过 0 失败**（新增 2 条），`verify.sh` / move 12 例 / 解释一致 / `jrust verify` 全通过；`jrust verify tests/l2/src` 通过。

## v1.17 — 2026-11-26

- **修复 `jrust verify <目录>` 的五处缺陷**（全部为真实工程/CI 环境才会暴露，v1.16 的用例一条都没踩到）：
  1. **cargo 没禁色导致错误映射整体失效**：`CARGO_TERM_COLOR=always` 时 `--> ` 带 ANSI 前缀，`scan_rustc_errors_multi` 全失配，所有报错退化为「未能定位到 .jrust 行」。修法：cargo 加 `--color never`，并让 `scan_rustc_errors` / `scan_rustc_errors_multi` 统一先过 `strip_ansi`。
  2. **manifest 从「抄原文件 + 剥 target 段」改为白名单重建**：只保留 `[package]` 的 name/version/edition 与依赖段，`[workspace]` / target 段 / features / profile 一律丢弃。修掉 `build = "build.rs"`、`default-run`、`[workspace]` + members、crate 内 `path` 依赖四类「必然失败且报错像用户代码」的问题；后两类改为明确诊断（`build` 与 `path` 直接拒绝，`workspace` 继承的 version/edition 回落默认值后可用）。
  3. **复用原 `Cargo.lock`**：此前临时目录会重新解析 `serde_json = "1"`，可能与用户实际编译版本不同，「与用户手工验收同一条路径」不成立。
  4. **`--offline` 失败自动在线重试一次**：依赖不在本地缓存时给出提示并重试，不再只抛 cargo 原文。
  5. **错误行号改报源码侧路径**：`错误: …/main.jrust:4`（此前报 `…/main.rs:4`，与 `.jrust` 行号混在一起）。
- **补齐 `verify_tree` 的验收**（此前该入口零覆盖，正是 v1.16「验收标准存在 ≠ 被执行」的同款问题）：新增 `run_verify_tree`（正向）/ `run_verify_tree_bad`（raw 内 E0599，断言映射回 `main.jrust:<行>`，并注入 `CARGO_TERM_COLOR=always` 复现 Agent/CI 环境）/ `run_verify_tree_manifest`（build 脚本必须给明确诊断）。做过变异验证：放开颜色控制后 `run_verify_tree_bad` 变红。
- 文档：`usage` / README 补目录模式与边界；SPEC §15 L1 计数表述修正、L2 行补 verify 边界；JR-G5 同步。
- 验证：L1 **119 通过 0 失败**（新增 3 条），`verify.sh` 全绿；六个手工场景（raw 报错 / build.rs / workspace / path 依赖 / default-run / `tests/l2/src`）逐个复测符合预期。

## v1.18 — 2026-11-26

- **examples 拆成 13 个独立 cargo 工程**：`examples/<Name>/{main.jrust, main.rs, Cargo.toml}`，
  `examples/Cargo.toml` 作为 workspace 共享一个 `target/`，bin 名 = 目录名小写。
  单独跑：`cargo run -p helloworld`，或 `cd examples/HelloWorld && cargo run`。
  每个目录也能直接 `jrust verify examples/<Name>`（v1.17 的目录模式正好派上用场）。
- **依赖不再寄生在空 crate 里**：`AsyncMain` 的 tokio 依赖落到自己的 `Cargo.toml`；
  删除只用于拼装编译的 `verify/` crate（`verify.sh` 不再复制 `.rs` 到 `verify/src/bin/`）。
- 脚本：`verify.sh` / `tests/interp_parity.sh` 改为遍历 `examples/*/main.jrust`，
  用 `cargo build --workspace` 一次编完；对拍直接执行 `examples/target/debug/<小写目录名>`。
- `.gitignore`：增 `/examples/target/`，删 verify/ 相关条目；`examples/Cargo.lock` 纳入版本管理。
- README：全部示例链接同步到 `examples/<Name>/main.jrust`，结构树与目录说明更新。
- 验证：`verify.sh` 全绿（L1 119 通过 0 失败）；`jrust verify examples/HelloWorld`、
  `cargo run -p helloworld`、`cargo run -p asyncmain` 三个入口逐一实测。

## v1.19 — 2026-11-26

- **修复回归：表达式位置的多行 `raw` 生成非法 Rust**。v1.16 为过 `rustfmt --check` 把
  `Expr::Raw` 从 `({ /* jrust:L */ body })` 改成 `(/* jrust:L */ body)`，块表达式外壳被丢；
  单行没问题，但多行 raw（体内含 `let` 等语句）会生成 `let y: i32 = (…let x = 1; x);`，
  rustc 报 `expected expression, found let statement`（旧形态可编）。
  现在：单行保持 `(/* jrust:L */ expr)`；多行回退块形态 `(/* jrust:L */{ body })`，
  体内语句按源缩进原样保留，收尾 `}` 对齐当前缩进。
- 补用例 `tests/conformance/L1/ok_raw_multiline.jrust`（+ `.expected`）：覆盖方法内多行 raw、
  `String` 返回值的多行 raw、`if` 内更深嵌套三处；用未修复的二进制复跑会失败（真实回归钉）。
- README `raw` 行补充形态说明；SPEC §15 L1 计数 119 → 120。
- 验证：`verify.sh` 全绿（L1 **120 通过 0 失败**，`fmt` 仍通过）。

## v1.20 — 2026-11-26

- **一致性用例支持目录式布局**（对齐 examples）：`tests/conformance/L1/<case>/{main.jrust, expected.txt}`。
  `tests/l1_cases.sh` 新增 `case_paths()`，统一解析 8 个 runner（`run_ok` / `run_neg` /
  `run_neg_plain` / `run_closure` / `run_build_has` / `run_build_lacks` / `run_warn` / `run_check`）
  的源码 / 产物 / 期望路径：**目录式优先，扁平 `ok_*.jrust` / `neg_*.jrust` 兼容**。
  存量 79 个用例一个没迁，零死链、零 diff 噪音。
- 首个目录式用例：`ok_raw_multiline`（v1.19 的扁平版迁入），作为新约定的样板；
  以后新增用例直接用目录，需要多文件/附属资产时也不用再改 runner。
- 生成的 `.rs` 仍就近落在用例旁边（已 gitignore），与 examples 一致，产物可对照。
- README 结构树补布局说明，`raw` 行的多行用例链接同步。
- 验证：`tests/l1_cases.sh` **120 通过 0 失败**（目录式与扁平式共存）。

## v1.21 — 2026-11-26

- **conformance/L1 全量目录化**：79 个用例统一为 `<case>/{main.jrust, expected.txt}`
  （负向用例只有 `main.jrust`）；78 次 `git mv` + 34 个 expected 改名，git 全部记为 rename，历史不丢。
- `run_verify` 也改走 `case_paths()` —— 此前是全脚本最后一处硬编码 `$DIR/$base.jrust`。
- README 32 处用例链接批量更新为 `tests/conformance/L1/<case>/main.jrust`；
  结构树说明改为「全部目录式，runner 仍兼容扁平历史写法」。
- 生成产物 `main.rs` 就近落在各用例目录，被 `.gitignore` 的 `/tests/conformance/**/*.rs` 覆盖。
- 验证：`tests/l1_cases.sh` **120 通过 0 失败**。

## v1.22 — 2026-11-26

### 修复（P1）

- **`-D warnings` 下产物编不过**：`raw` 外层的来源映射括号会触发 `unused_parens`，
  生成头的 `allow` 只有 4 项。补上 `unused_parens`（`src/codegen.rs`），并新增
  `run_strict_warnings` 用例（对 `ok_raw` 产物跑 `rustc -D warnings`，变异验证过灵敏度）。
- **`verify <目录>` 丢 `[[bin]]`**：白名单重建 manifest 时把原 `[[bin]]` 全丢、统一改名
  `jrust-verify-tree`。现在把 `path` 能映射到待验目录内、且确实是 jrust 产物的 bin 按原名保留；
  手写 `.rs` / 目录外目标跳过（它们在临时工程里无法重建）。
- **lib-only 多文件工程不能验**：`plan_tree` 在「没有含 main 的根」时直接报错。现在按 lib crate
  处理（根产物 `lib.rs`，顶层包 `pub mod` 自动写入），与单文件 `verify --crate-type lib` 对齐；
  `jrust build <目录>` 同样受益。新增 `run_verify_tree_lib`（正向 + 负向映射到 `.jrust` 行）。

### 澄清（不是 bug）

- **「多行 raw 破坏 rustfmt --check」不成立**：复核发现产物本来就不做 rustfmt 归一化
  （13 个例程里 4 个因长行/结构体字面量本来就不 fmt-clean），多行 raw 的缩进与生成嵌套
  1:1 对应，`rustfmt --check` 实测通过。因此不改缩进（改了反而会破坏多行字符串字面量），
  而是修正文档口径：SPEC **JR-G9** 由「产物必须通过 rustfmt --check」改为
  「不追求整份 fmt-clean，但必须能编译、且在 -D warnings 下干净」。

### 验收

`./verify.sh` 全绿：L1 **122 通过 0 失败**，新增 `vtree_lib` / `strict_warnings` 两项。

## v1.23 — 2026-11-26

### 修复（P2 边角）

- **错误行扫描张冠李戴**：`scan_rustc_errors_multi` 的 `pending` 只在下一次 `-->` 时清空，
  导致「无定位的 error（如 `aborting due to N previous errors`）」会把随后 warning 的
  `-->` 当成自己的位置，错误信息与行号错配。现在 `warning:` / `note:` 与 cargo 汇总行
  都会清空 `pending`。
- **离线重试判据偏窄**：原判据只看 `--offline` / `no matching package`，
  `registry index was not found`、`failed to get ... as a dependency` 等不会重试。
  抽出 `is_resolve_issue()`：命中解析/索引类文案且**不含 `error[E…]`** 才在线重试一次，
  避免把真正的编译错误重跑一遍。
- **examples workspace 成员手工维护**：`members` 改为 `["*"]` + `exclude = ["target"]`
  （cargo 的 glob 不按「有没有 Cargo.toml」过滤，构建产物目录必须显式排除）。
  新增例程只需建目录 + `Cargo.toml`，实测临时 `ZzProbe` 被自动纳入并可 `-p` 构建。
- **顺带：根 crate 的 `autoexamples = false`**。`examples/` 是独立的 cargo 工程集合，
  但 cargo 会把它当成本 crate 的 `Cargo` example 自动发现 —— `cargo test` 因此把
  `AsyncMain` 当示例编译、因缺 tokio 失败。关掉自动发现后 `cargo test` 才可用。

### 验收

`verify.sh` 增加第 2 步 `cargo test`（4 个新增单测：warning 不配 error 位置、
正常配对、ANSI 剥离、离线判据分类），总计 **8 个单测全绿**；
L1 **122 通过 0 失败**。

## v1.24 — 2026-11-26

### 改进（P2 收尾）

- **跳过 `[[bin]]` 时不再静默**：`verify <目录>` 重建 manifest 时，指向手写 `.rs` /
  待验目录之外的 bin 会打印
  `提示: 跳过 N 个 [[bin]]（不在待验目录内，或不是 jrust 产物）：<path>...`。
  `run_verify_tree` 增加断言：l2 的 `ref/reference.rs` 必须出现在提示里。
- **`./verify.sh` 覆盖全套**：由 5 步扩到 **7 步**，补上 `tests/move_cases.sh`
  （12 个 move 非法边界）与 `tests/interp_parity.sh`（解释 / 编译输出对拍）。
  此前这两项只在 `jrust verify`（无参数）里跑，「端到端脚本 ≠ 全套」是个隐性缺口。
- README 的 `./verify.sh` 说明同步为七步。

### 验收

`./verify.sh` 七步全绿：cargo test 8 passed、13 例程 cargo build、L1 122 通过 0 失败、
move 12 项全过、对拍输出一致。

## v1.25 — 2026-11-26

### 修复（L2 残留复核的 P0/P1/P2/P3 全量落地）

- **P0 根因修复：类型表提升到工程级**。`Codegen` 新增 `types` 字段（`with_types`），
  与 `prog`（决定输出哪些声明）分离；`generate_multi_with_sigs` 增加 `types` 参数，
  `main.rs` 用新的 `merge_programs` 合并全工程后传入。一次修掉 7 条跨文件缺陷：
  ① 构造器字面量实参不补 `.to_string()`；② 实例方法 `Value` 形参同样；
  ③ 静态方法（原本靠多文件启发式侥幸正常，现在两条路径统一）；④ `String + String`
  不生成 `format!`；⑤ 接口作类型不包 `Box<dyn>`；⑥ enum 误判不可 Clone（误报 JR4002）；
  ⑦ 可空返回双重 Option。
- **P1 `list.get` owned 上下文**：`infer_type` 认识 `list.get(i)` 的元素类型；
  新增 `needs_owned_landing`，`clone_if_needed` 对 `x.get(..)` 补落地转换。
  此前 `String s = xs.get(0)` → `xs[0]` → `E0507`。
- **P1 依赖解析统一**：新增 `parse_manifest_deps` / `dep_section_kind` / `DepDecl`，
  JR-M4 校验与 `verify <目录>` 的 manifest 重建共用同一解析器。修掉
  `[dependencies.serde_json]` 校验认、重建丢（→「校验通过但 cargo 说没这个 crate」），
  以及 `[target.'cfg(unix)'.dependencies.foo]` 两边都不认（→ 误报 JR5001）。
- **P1 关键字转义（JR-L1 落地）**：`rust_ident` 对 41 个 Rust 关键字加 `r#`，
  覆盖类型名 / 模块段 / 变量名 / import 路径；`crate` / `self` / `Self` / `super`
  无法转义，报新诊断 **`JR3030`**。parser 里 `package` / `import` 路径遇到关键字
  不再静默跳过（此前生成 `crate::::thing::Thing`）。
- **P2 `-D warnings`**：生成头 allow 补 `non_snake_case`（Java 驼峰命名必然触发）与
  `unused_assignments`（先声明后赋值的生成形态），兑现 JR-G9。
  `run_strict_warnings` 从 1 个用例扩到 2 个 conformance + 3 个例程。
- **P2 lib-only 根文件**：无 package 且无 main 的文件现在作为 lib crate 根（可放
  `pub struct`）；多个这种文件报错。此前 `root_out` 固定成自动生成的 `lib.rs`，
  用户两条路都堵死。
- **P2 错误展示**：按 `(路径, 行, 消息)` 去重（同列位重复条目）；映射不到源行时
  明确说「位于文件头部的 import / 属性区」而不是只报 `:0`；单文件模式误用于多文件
  工程时提示改用目录模式。
- **P3 workspace lockfile**：新增 `find_lockfile` 向上查找，workspace 成员的
  `verify` 也能用根目录的 `Cargo.lock`（此前只查 `manifest.parent()`）。

### 验收

新增夹具 `tests/l2_cross/`（跨文件矩阵：7 个维度 × 字面量/变量，双 bin 与手写
参考实现对拍）；新增 4 个 L2 用例（`l2_cross` / `l2_ctor` / `l2_null` / `l2_iface`）
+ `vtree_depforms` + 2 个 conformance（`ok_list_get_owned` / `ok_keyword_escape`）。
L1 **129 通过 0 失败**，`./verify.sh` 七步全绿。
**变异验证**：把类型表退回单文件 → 4 条 L2 用例变红；去掉 `list.get` 落地转换 /
关键字转义 / warning allow / 依赖段统一 → 对应用例各自变红。

## v1.26 — 2026-11-26

### 修复

- **`switch` 未覆盖全部枚举分支的行号错位**：`gen_switch` 先把 `cur_line` 推到
  最后一个 case 体的末行，再报 JR3019，导致诊断指向 case 内的语句而不是 `switch`。
  现在 `gen_switch` 接收 `line`，缺分支错误报在 `switch` 行本身
  （`neg_switch_incomplete` 由 11 行纠正为 9 行）。
- **30 个负向用例补上行号断言**：`run_neg` 的第 4 个参数（期望行号）此前一个都没传，
  等于「行号错位」这类回归无法被测试发现。逐条核对实际报错位置确实落在出错构造上后
  全部补上；`run_neg` 现在会断言 `:<行> - error[<码>]` 的精确形态。

### 验收

`./verify.sh` 七步全绿；L1 **129 通过 0 失败**（用例数不变，断言变严）。
变异验证：把 `gen_switch` 的 `line` 参数回退（继续用 `cur_line`）→ `neg_switch_incomplete`
立即变红。

## v1.27 — 2026-11-26

### 测试

- **解释 / 编译对拍扩到 L1 夹具**：`tests/interp_parity.sh` 原来只跑 13 个 examples，
  L1 的 79 个夹具（raw、泛型上界、throws、`T?`、C 风格 `for` 等）只在编译侧验过。
  现在遍历全部带 `expected.txt` 的 36 个正向夹具：能解释执行的要求与期望输出逐字节一致
  （当前 **18 个**），解释器跑不动的必须出现在新清单 `tests/interp_unsupported.txt` 里
  才允许 SKIP（当前 **18 个**，逐条注明缺什么能力）。
  - 清单外的用例一旦跑不动 → 直接 FAIL，避免「解释器回归」被当成「不支持」静默跳过；
  - 清单内的用例哪天能跑了 → 打印「可移除」提示，让覆盖自己长起来。
- **`jrust check` 全量覆盖**：新增 `run_check_bulk`，36 个正向夹具必须 check 通过、
  36 个 `neg_*` 夹具必须 check 报错；逐条跑但只计 2 项（失败时列出具体夹具，不灌水
  用例总数）。此前 check 只有 2 个手挑用例。

### 验收

`./verify.sh` 七步全绿，L1 **131 通过 0 失败**（新增 `check:bulk_positive` /
`check:bulk_negative`）；对拍「examples 13 + L1 18，清单跳过 18」。
变异验证：往 `ok_assert/expected.txt` 追加一行 → 对拍 FAIL；把一个负向夹具伪装成正向 →
`check:bulk_positive` FAIL 并点名该夹具。
