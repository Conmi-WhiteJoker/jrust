#!/usr/bin/env bash
# 端到端验证：jrust 转译 examples/*/main.jrust，再用 cargo build 编译生成的 Rust。
set -euo pipefail
cd "$(dirname "$0")"

echo "== 1/11 构建 jrust =="
cargo build --quiet

echo "== 2/11 单元测试（rustc 错误行扫描 / 离线重试判据）=="
cargo test --quiet

echo "== 3/11 转译 examples/*/main.jrust =="
for f in examples/*/main.jrust; do
    ./target/debug/jrust build "$f"
done

echo "== 4/11 cargo build 验证生成的 Rust（examples 是 workspace，15 个例程各为独立 crate） =="
cargo build --quiet --manifest-path examples/Cargo.toml --workspace

echo "== 5/11 L1 一致性用例（throws / switch / enum / 闭包 / T? / for / new T[n] / 字符串 + / 泛型类·方法·接口·枚举·上界）=="
./tests/l1_cases.sh

echo "== 6/11 move 非法边界用例 =="
./tests/move_cases.sh

echo "== 7/11 解释执行 vs 编译执行 输出对拍 =="
./tests/interp_parity.sh

echo "== 8/11 L3 async 小服务（axum + 共享状态 + 优雅退出，压测 ≥1k QPS） =="
./tests/l3_smoke.sh

echo "== 9/11 L4 async 中型服务（多模块 + 外部 crate + 后台任务；与手写版本逐字节对拍，含 3 条失败路径） =="
./tests/l4_cases.sh

echo "== 10/11 产物快照（阶段 A 硬门槛：重构不得改变任何既有产物） =="
./tests/snapshot.sh --check

echo "== 11/11 附录 A 覆盖率盘点（每个 ✅ 需求都有夹具验证） =="
./tests/coverage_audit.sh

echo "全部通过：jrust 产物可被 rustc 编译。"
