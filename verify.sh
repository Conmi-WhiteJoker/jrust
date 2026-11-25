#!/usr/bin/env bash
# 端到端验证：jrust 转译 examples/*.jrust，再用 cargo build 编译生成的 Rust。
set -euo pipefail
cd "$(dirname "$0")"

echo "== 1/4 构建 jrust =="
cargo build --quiet

echo "== 2/4 转译 examples/*.jrust =="
mkdir -p verify/src/bin
rm -f verify/src/bin/*.rs
for f in examples/*.jrust; do
    ./target/debug/jrust build "$f"
    base=$(basename "${f%.jrust}")
    cp "${f%.jrust}.rs" "verify/src/bin/${base}.rs"
done

echo "== 3/4 cargo build 验证生成的 Rust =="
cargo build --quiet --manifest-path verify/Cargo.toml

echo "== 4/4 L1 一致性用例（throws / switch / enum / 闭包 / T? / for / new T[n] / 字符串 + / 泛型类·方法·接口·枚举）=="
./tests/l1_cases.sh

echo "全部通过：jrust 产物可被 rustc 编译。"
