#!/usr/bin/env bash
# 端到端验证：jrust 转译 examples/*.jrust，再用 cargo build 编译生成的 Rust。
set -euo pipefail
cd "$(dirname "$0")"

echo "== 1/3 构建 jrust =="
cargo build --quiet

echo "== 2/3 转译 examples/*.jrust =="
mkdir -p verify/src/bin
rm -f verify/src/bin/*.rs
for f in examples/*.jrust; do
    ./target/debug/jrust build "$f"
    base=$(basename "${f%.jrust}")
    cp "${f%.jrust}.rs" "verify/src/bin/${base}.rs"
done

echo "== 3/3 cargo build 验证生成的 Rust =="
cargo build --quiet --manifest-path verify/Cargo.toml

echo "全部通过：jrust 产物可被 rustc 编译。"
