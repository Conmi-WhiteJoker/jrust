#!/usr/bin/env bash
# L1 一致性用例：throws/Result/?、switch→match、带数据 enum、闭包、T?/null、C 风格 for、new T[n]、字符串 + 拼接，以及对应诊断码。
set -euo pipefail
cd "$(dirname "$0")/.."

JRUST=./target/debug/jrust
DIR=tests/conformance/L1
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

fail=0
pass=0

# 正向用例：转译 → rustc 编译 → 运行 → 比对输出
run_ok() {
    local base=$1
    local src="$DIR/$base.jrust"
    if ! $JRUST build "$src" >/dev/null 2>"$TMP/err"; then
        echo "FAIL $base：转译失败"
        cat "$TMP/err"
        fail=$((fail + 1))
        return
    fi
    if ! rustc --edition 2021 -A warnings "$DIR/$base.rs" -o "$TMP/$base" 2>"$TMP/err"; then
        echo "FAIL $base：生成的 Rust 编译失败"
        cat "$TMP/err"
        fail=$((fail + 1))
        return
    fi
    "$TMP/$base" >"$TMP/out"
    if diff -u "$DIR/$base.expected" "$TMP/out" >"$TMP/diff"; then
        echo "PASS $base"
        pass=$((pass + 1))
    else
        echo "FAIL $base：输出不一致"
        cat "$TMP/diff"
        fail=$((fail + 1))
    fi
}

# 负向用例：必须失败，且输出包含指定诊断码 / 提示
run_neg() {
    local base=$1
    local code=$2
    local pattern=$3
    if $JRUST build "$DIR/$base.jrust" >"$TMP/out" 2>&1; then
        echo "FAIL $base：本应报错却成功了"
        fail=$((fail + 1))
        return
    fi
    if ! grep -q "error\[$code\]" "$TMP/out"; then
        echo "FAIL $base：缺少诊断码 $code"
        cat "$TMP/out"
        fail=$((fail + 1))
        return
    fi
    if ! grep -q "提示:" "$TMP/out" || ! grep -q -- "$pattern" "$TMP/out"; then
        echo "FAIL $base：提示行不符合预期（$pattern）"
        cat "$TMP/out"
        fail=$((fail + 1))
        return
    fi
    echo "PASS $base"
    pass=$((pass + 1))
}

# 闭包目前没有可表达的函数类型，只校验产物形态。
run_closure() {
    local base=ok_closure_transpile
    if ! $JRUST build "$DIR/$base.jrust" >/dev/null 2>&1; then
        echo "FAIL $base：转译失败"
        fail=$((fail + 1))
        return
    fi
    if grep -q '|x| x \* 2' "$DIR/$base.rs" \
        && grep -q '|x| x + 1' "$DIR/$base.rs" \
        && grep -q '|a: f64, b: f64| a + b' "$DIR/$base.rs"; then
        echo "PASS $base"
        pass=$((pass + 1))
    else
        echo "FAIL $base：产物里没有预期的闭包写法"
        fail=$((fail + 1))
    fi
}

# 只校验产物形态（需要外部运行时、无法用 rustc 直接编译运行的特性）
run_build_has() {
    local base=$1
    local pattern=$2
    if ! $JRUST build "$DIR/$base.jrust" >/dev/null 2>"$TMP/err"; then
        echo "FAIL $base：转译失败"
        cat "$TMP/err"
        fail=$((fail + 1))
        return
    fi
    if grep -q -- "$pattern" "$DIR/$base.rs"; then
        echo "PASS $base"
        pass=$((pass + 1))
    else
        echo "FAIL $base：产物里没有预期写法（$pattern）"
        fail=$((fail + 1))
    fi
}

run_ok ok_throws
run_ok ok_enum_switch
run_ok ok_bool_switch
run_ok ok_nullable
run_ok ok_cfor
run_ok ok_newarray
run_ok ok_concat
run_ok ok_generic
run_ok ok_generic_iface_enum
run_ok ok_generic_bounds
run_ok ok_const
run_ok ok_attrs
run_ok ok_assert
run_ok ok_async
run_ok ok_raw
run_ok ok_iface_default
run_ok ok_iter_chain
run_ok ok_shared
run_ok ok_sync
run_closure
run_build_has ok_spawn 'tokio::spawn(async move {'
run_build_has ok_async_main '#\[tokio::main\]'
run_build_has ok_async_main 'async fn main'

run_neg neg_try_without_throws JR3018 "给当前方法签名加"
run_neg neg_switch_int JR3019 "int / String"
run_neg neg_switch_incomplete JR3019 "补上缺少的 case"
run_neg neg_fallthrough JR3020 "给该分支补"
run_neg neg_throw_without_throws JR3021 "在方法签名后加"
run_neg neg_throw_undeclared JR3021 "在方法签名里声明"
run_neg neg_null_assign JR3005 "改用一个非空值"
run_neg neg_nullable_method JR3006 "先判空再取出来"
run_neg neg_cfor_step JR2003 "步长不为 1"
run_neg neg_cfor_dir JR2003 "方向不一致"
run_neg neg_newarray_type JR3014 "只支持标量 / String"
run_neg neg_wildcard JR3010 "trait object"
run_neg neg_mutable_static JR3013 "可变全局"
run_neg neg_await_sync JR3022 "async"
run_neg neg_spawn_this JR3023 "this"
run_neg neg_iface_static_no_body JR3024 "必须有方法体"
run_neg neg_iter_unknown JR3017 "白名单"
run_neg neg_sync_target JR3026 "this.f"
run_neg neg_labeled JR2004 "不支持带标签"
run_neg neg_raw_nested JR3027 "不能嵌套"

echo "L1 用例：$pass 通过，$fail 失败"
[ "$fail" -eq 0 ]
