#!/usr/bin/env bash
# 端到端验证：jrust 转译 examples/*/main.jrust，再用 cargo build 编译生成的 Rust。
set -euo pipefail
cd "$(dirname "$0")"

echo "== 1/7 构建 jrust =="
cargo build --quiet

echo "== 2/7 单元测试（rustc 错误行扫描 / 离线重试判据）=="
cargo test --quiet

echo "== 3/7 转译 examples/*/main.jrust =="
for f in examples/*/main.jrust; do
    ./target/debug/jrust build "$f"
done

echo "== 4/7 cargo build 验证生成的 Rust（examples 是 workspace，13 个例程各为独立 crate） =="
cargo build --quiet --manifest-path examples/Cargo.toml --workspace

echo "== 5/7 L1 一致性用例（throws / switch / enum / 闭包 / T? / for / new T[n] / 字符串 + / 泛型类·方法·接口·枚举·上界）=="
./tests/l1_cases.sh

echo "== 6/7 move 非法边界用例 =="
./tests/move_cases.sh

echo "== 7/7 解释执行 vs 编译执行 输出对拍 =="
./tests/interp_parity.sh

echo "全部通过：jrust 产物可被 rustc 编译。"
