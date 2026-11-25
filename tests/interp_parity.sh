#!/usr/bin/env bash
# 解释执行 vs 编译执行 对拍：
#   对每个 examples/*.jrust，比较 `jrust run` 与「转译后 cargo 跑出来的二进制」的 stdout。
# 两者必须逐字节一致——这是解释器没跑偏的硬指标。
set -euo pipefail
cd "$(dirname "$0")/.."

echo "== 1/4 构建 jrust =="
cargo build --quiet

echo "== 2/4 转译 examples/*.jrust =="
mkdir -p verify/src/bin
rm -f verify/src/bin/*.rs
for f in examples/*.jrust; do
    ./target/debug/jrust build "$f" >/dev/null
    base=$(basename "${f%.jrust}")
    cp "${f%.jrust}.rs" "verify/src/bin/${base}.rs"
done

echo "== 3/4 编译转译产物 =="
cargo build --quiet --manifest-path verify/Cargo.toml

echo "== 4/4 对拍 stdout =="
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

fail=0
for f in examples/*.jrust; do
    base=$(basename "${f%.jrust}")
    if ! ./target/debug/jrust run "$f" >"$tmp/${base}.interp" 2>"$tmp/${base}.interp.err"; then
        echo "FAIL $base: jrust run 退出码非零"
        cat "$tmp/${base}.interp.err"
        fail=1
        continue
    fi
    cargo run --quiet --manifest-path verify/Cargo.toml --bin "$base" >"$tmp/${base}.rust" 2>/dev/null
    if diff -u "$tmp/${base}.interp" "$tmp/${base}.rust" >"$tmp/${base}.diff"; then
        echo "PASS $base"
    else
        echo "FAIL $base: 解释执行与编译执行输出不一致"
        cat "$tmp/${base}.diff"
        fail=1
    fi
done

if [ "$fail" -ne 0 ]; then
    echo "对拍失败。"
    exit 1
fi
echo "全部通过：解释执行与编译执行输出一致。"
