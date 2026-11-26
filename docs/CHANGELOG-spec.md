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
- **JR-G5**（verify）：`jrust verify` 不带参数时依次运行 `tests/l1_cases.sh`、`tests/move_cases.sh`、`tests/interp_parity.sh`；`fmt` 仍待做。
