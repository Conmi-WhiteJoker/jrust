#!/usr/bin/env bash
# 解释执行 vs 编译执行 对拍：
#   对每个 examples/*/main.jrust，比较 `jrust run` 与「转译后 cargo 跑出来的二进制」的 stdout。
# 两者必须逐字节一致——这是解释器没跑偏的硬指标。
set -euo pipefail
cd "$(dirname "$0")/.."

echo "== 1/4 构建 jrust =="
cargo build --quiet

echo "== 2/4 转译 examples/*/main.jrust =="
for f in examples/*/main.jrust; do
    ./target/debug/jrust build "$f" >/dev/null
done

echo "== 3/4 编译转译产物 =="
cargo build --quiet --manifest-path examples/Cargo.toml --workspace

echo "== 4/4 对拍 stdout =="
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

fail=0
skip=0
ex_pass=0
for f in examples/*/main.jrust; do
    base=$(basename "$(dirname "$f")")
    if ! ./target/debug/jrust run "$f" >"$tmp/${base}.interp" 2>"$tmp/${base}.interp.err"; then
        echo "FAIL $base: jrust run 退出码非零"
        cat "$tmp/${base}.interp.err"
        fail=1
        continue
    fi
    # 每个例程是独立 crate，bin 名 = 目录名小写（见 examples/<Name>/Cargo.toml）。
    bin=$(echo "$base" | tr '[:upper:]' '[:lower:]')
    "examples/target/debug/$bin" >"$tmp/${base}.rust" 2>/dev/null
    if diff -u "$tmp/${base}.interp" "$tmp/${base}.rust" >"$tmp/${base}.diff"; then
        echo "PASS $base"
        ex_pass=$((ex_pass + 1))
    else
        echo "FAIL $base: 解释执行与编译执行输出不一致"
        cat "$tmp/${base}.diff"
        fail=1
    fi
done
l1_pass=0
for e in tests/conformance/L1/*/expected.txt; do
    d=$(dirname "$e")
    base=$(basename "$d")
    if ./target/debug/jrust run "$d/main.jrust" >"$tmp/$base.out" 2>"$tmp/$base.err"; then
        # 已列在「暂不支持」清单里但能跑了：提示移除，不阻断。
        if grep -q "^$base\s" tests/interp_unsupported.txt 2>/dev/null; then
            echo "提示 $base: 解释执行已可用，可从 tests/interp_unsupported.txt 移除"
        fi
        if diff -u "$e" "$tmp/$base.out" >"$tmp/$base.diff"; then
            echo "PASS L1/$base"
            l1_pass=$((l1_pass + 1))
        else
            echo "FAIL L1/$base: 解释执行与期望输出不一致"
            cat "$tmp/$base.diff"
            fail=1
        fi
    elif grep -q "^$base\s" tests/interp_unsupported.txt 2>/dev/null; then
        echo "SKIP L1/$base（在 tests/interp_unsupported.txt 中）"
        skip=$((skip + 1))
    else
        echo "FAIL L1/$base: 解释执行失败（清单里没有它）"
        cat "$tmp/$base.err"
        fail=1
    fi
done

if [ "$fail" -ne 0 ]; then
    echo "对拍失败。"
    exit 1
fi
echo "全部通过：解释执行与编译执行输出一致（examples $ex_pass 个 + L1 $l1_pass 个，清单跳过 $skip 个）。"
