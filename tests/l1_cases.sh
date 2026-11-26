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

# `jrust check`：只做检查、不写产物；ok 期望退出 0，fail 期望非 0。
run_check() {
    local base=$1
    local expect=$2
    if [ "$expect" = ok ]; then
        if $JRUST check "$DIR/$base.jrust" >"$TMP/out" 2>&1; then
            echo "PASS check:$base"
            pass=$((pass + 1))
        else
            echo "FAIL check:$base：本应通过"
            cat "$TMP/out"
            fail=$((fail + 1))
        fi
    else
        if $JRUST check "$DIR/$base.jrust" >"$TMP/out" 2>&1; then
            echo "FAIL check:$base：本应报错"
            fail=$((fail + 1))
        else
            echo "PASS check:$base"
            pass=$((pass + 1))
        fi
    fi
}


# 多文件模式（JR-S4/M3）：package → 模块树、crate 根、crate:: 前缀的 use。
run_multifile() {
    local work="$TMP/multifile"
    rm -rf "$work"
    cp -r tests/multifile "$work"
    if ! $JRUST build "$work" >"$TMP/out" 2>&1; then
        echo "FAIL multifile：转译失败"
        cat "$TMP/out"
        fail=$((fail + 1))
        return
    fi
    if [ ! -f "$work/app/greet.rs" ] || [ ! -f "$work/app/mod.rs" ]; then
        echo "FAIL multifile：缺少 app/greet.rs 或 app/mod.rs"
        fail=$((fail + 1))
        return
    fi
    if ! grep -q 'use crate::app::greet::Greeter;' "$work/main.rs"; then
        echo "FAIL multifile：main.rs 的 use 缺少 crate:: 前缀"
        fail=$((fail + 1))
        return
    fi
    if ! grep -q 'pub mod app;' "$work/main.rs" || ! grep -q 'pub mod greet;' "$work/app/mod.rs"; then
        echo "FAIL multifile：模块声明缺失"
        fail=$((fail + 1))
        return
    fi
    if ! rustc --edition 2021 -A warnings "$work/main.rs" -o "$TMP/mf" 2>"$TMP/err"; then
        echo "FAIL multifile：生成的 Rust 编译失败"
        cat "$TMP/err"
        fail=$((fail + 1))
        return
    fi
    "$TMP/mf" >"$TMP/out"
    if [ "$(cat "$TMP/out")" = "Hello, jrust" ]; then
        echo "PASS multifile"
        pass=$((pass + 1))
    else
        echo "FAIL multifile：输出是 $(cat "$TMP/out")"
        fail=$((fail + 1))
    fi
}

# 含 main 的文件声明 package → JR3029，提示必须是 crate 根。
run_multifile_neg() {
    local work="$TMP/multifile_bad"
    rm -rf "$work"
    cp -r tests/multifile_bad "$work"
    if $JRUST build "$work" >"$TMP/out" 2>&1; then
        echo "FAIL multifile_bad：本应报错却成功了"
        fail=$((fail + 1))
        return
    fi
    if grep -q 'error\[JR3029\]' "$TMP/out" && grep -q 'crate 根' "$TMP/out"; then
        echo "PASS multifile_bad"
        pass=$((pass + 1))
    else
        echo "FAIL multifile_bad：缺少 JR3029 / crate 根提示"
        cat "$TMP/out"
        fail=$((fail + 1))
    fi
}

# JR-M4 / JR-C15：外部 crate 必须在最近的 Cargo.toml 声明，缺失报 JR5001。
run_deps() {
    local src=$1
    local expect=$2
    local pattern=${3:-}
    if $JRUST check "$src" >"$TMP/out" 2>&1; then
        if [ "$expect" = ok ]; then
            echo "PASS deps:$src"
            pass=$((pass + 1))
        else
            echo "FAIL deps:$src：本应报错却通过了"
            fail=$((fail + 1))
        fi
    else
        if [ "$expect" = fail ] && grep -q 'error\[JR5001\]' "$TMP/out" && grep -q -- "$pattern" "$TMP/out"; then
            echo "PASS deps:$src"
            pass=$((pass + 1))
        else
            echo "FAIL deps:$src：缺少 JR5001 / $pattern"
            cat "$TMP/out"
            fail=$((fail + 1))
        fi
    fi
}

# JR-C14 / JR-X2：verify 交给 rustc 编译，并把报错映射回 .jrust 行（raw 块内附原文行号）。
run_verify() {
    local base=$1
    local expect=$2
    local pattern=${3:-}
    if $JRUST verify "$DIR/$base.jrust" >"$TMP/out" 2>&1; then
        if [ "$expect" = ok ]; then
            echo "PASS verify:$base"
            pass=$((pass + 1))
        else
            echo "FAIL verify:$base：本应报错却通过了"
            fail=$((fail + 1))
        fi
    else
        if [ "$expect" = fail ] && grep -q 'error\[E0277\]' "$TMP/out" && grep -q -- "$pattern" "$TMP/out"; then
            echo "PASS verify:$base"
            pass=$((pass + 1))
        else
            echo "FAIL verify:$base：缺少 E0277 / $pattern"
            cat "$TMP/out"
            fail=$((fail + 1))
        fi
    fi
}

# JR-G5 fmt：统一 4 空格缩进，raw 块逐字保留；格式化后仍能编译运行。
run_fmt() {
    local work="$TMP/fmt"
    rm -rf "$work"
    cp -r tests/fmt "$work"
    if ! $JRUST fmt "$work/main.jrust" >"$TMP/out" 2>&1; then
        echo "FAIL fmt：格式化失败"
        cat "$TMP/out"
        fail=$((fail + 1))
        return
    fi
    if ! grep -q '^    raw {' "$work/main.jrust" \
        || ! grep -q '^    public static void main' "$work/main.jrust" \
        || ! grep -q '^        System.out.println' "$work/main.jrust" \
        || ! grep -qF '   fn helper() { println!("raw keep"); }' "$work/main.jrust"; then
        echo "FAIL fmt：缩进或 raw 保留不符合预期"
        cat "$work/main.jrust"
        fail=$((fail + 1))
        return
    fi
    if ! $JRUST build "$work/main.jrust" >"$TMP/out" 2>&1; then
        echo "FAIL fmt：格式化后转译失败"
        cat "$TMP/out"
        fail=$((fail + 1))
        return
    fi
    if ! rustc --edition 2021 -A warnings "$work/main.rs" -o "$TMP/fmtbin" 2>"$TMP/err"; then
        echo "FAIL fmt：格式化后编译失败"
        cat "$TMP/err"
        fail=$((fail + 1))
        return
    fi
    "$TMP/fmtbin" >"$TMP/out"
    if [ "$(cat "$TMP/out")" = "ok" ]; then
        echo "PASS fmt"
        pass=$((pass + 1))
    else
        echo "FAIL fmt：输出是 $(cat "$TMP/out")"
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
run_ok ok_docs
run_ok ok_set
run_ok ok_for_mutable
run_ok ok_iface_extends
run_ok ok_illegal_arg
run_ok ok_custom_exception
run_ok ok_mut_receiver
run_closure
run_build_has ok_spawn 'tokio::spawn(async move {'
run_build_has ok_async_main '#\[tokio::main\]'
run_build_has ok_async_main 'async fn main'
run_build_has ok_illegal_arg 'panic!'
run_build_has ok_custom_exception 'pub struct AppError'
run_build_has ok_custom_exception 'impl std::error::Error for AppError'
run_build_has ok_mut_receiver 'fn inc(&mut self)'
run_build_has ok_mut_receiver 'fn get(&self)'

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
run_neg neg_mutable_set JR3028 "只适用于 List"
run_neg neg_return_this JR4003 "静态工厂"

run_check ok_shared ok
run_check neg_labeled fail
run_multifile
run_multifile_neg
run_deps tests/deps_bad/bad.jrust fail 'serde_json'
run_deps tests/deps_ok/ok.jrust ok
run_deps tests/deps_async/src/main.jrust fail 'tokio'
run_verify verify_ok ok
run_verify verify_raw_error fail 'raw 块内'
run_fmt

echo "L1 用例：$pass 通过，$fail 失败"
[ "$fail" -eq 0 ]
