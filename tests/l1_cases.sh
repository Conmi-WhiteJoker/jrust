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

# 用例布局：目录式 `L1/<case>/{main.jrust, expected.txt}` 优先，扁平 `L1/<case>.jrust` 兼容。
# 调用后可用 CASE_SRC（源码）/ CASE_RS（产物，就近生成）/ CASE_EXPECTED（期望 stdout）。
case_paths() {
    local base=$1
    if [ -d "$DIR/$base" ]; then
        CASE_SRC="$DIR/$base/main.jrust"
        CASE_RS="$DIR/$base/main.rs"
        CASE_EXPECTED="$DIR/$base/expected.txt"
    else
        CASE_SRC="$DIR/$base.jrust"
        CASE_RS="$DIR/$base.rs"
        CASE_EXPECTED="$DIR/$base.expected"
    fi
}

# 正向用例：转译 → rustc 编译 → 运行 → 比对输出
run_ok() {
    local base=$1
    case_paths "$base"
    local src="$CASE_SRC"
    if ! $JRUST build "$src" >/dev/null 2>"$TMP/err"; then
        echo "FAIL $base：转译失败"
        cat "$TMP/err"
        fail=$((fail + 1))
        return
    fi
    if ! rustc --edition 2021 -A warnings "$CASE_RS" -o "$TMP/$base" 2>"$TMP/err"; then
        echo "FAIL $base：生成的 Rust 编译失败"
        cat "$TMP/err"
        fail=$((fail + 1))
        return
    fi
    "$TMP/$base" >"$TMP/out"
    if diff -u "$CASE_EXPECTED" "$TMP/out" >"$TMP/diff"; then
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
    local line=${4:-}
    case_paths "$base"
    if $JRUST build "$CASE_SRC" >"$TMP/out" 2>&1; then
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
    # 可选第 4 参数：断言行号，格式必须是 `:<行>:0: error[<码>]`（JR-G7，v1 列号固定 0）。
    if [ -n "$line" ] && ! grep -q ":$line:0: error\[$code\]" "$TMP/out"; then
        echo "FAIL $base：诊断行号不是 $line"
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

# parser 层错误（没有诊断码）：必须失败，且输出同时包含消息片段与提示片段。
# `jrust check`（编辑器保存钩子入口）的全量覆盖：
#   所有正向夹具必须 check 通过，所有负向夹具必须 check 报错。
# 逐条跑但只计 2 项，失败时列出具体夹具，避免用例总数被灌水。
run_check_bulk() {
    local bad="" n_ok=0 n_neg=0 d base
    for e in "$DIR"/*/expected.txt; do
        d=$(dirname "$e")
        if $JRUST check "$d/main.jrust" >"$TMP/out" 2>&1; then
            n_ok=$((n_ok + 1))
        else
            bad="$bad 正向:$(basename "$d")"
        fi
    done
    if [ -z "$bad" ]; then
        echo "PASS check:bulk_positive（$n_ok 个）"
        pass=$((pass + 1))
    else
        echo "FAIL check:bulk_positive：$bad"
        fail=$((fail + 1))
    fi
    bad=""
    for d in "$DIR"/neg_*/; do
        if $JRUST check "$d/main.jrust" >"$TMP/out" 2>&1; then
            bad="$bad 负向误过:$(basename "$d")"
        else
            n_neg=$((n_neg + 1))
        fi
    done
    if [ -z "$bad" ]; then
        echo "PASS check:bulk_negative（$n_neg 个）"
        pass=$((pass + 1))
    else
        echo "FAIL check:bulk_negative：$bad"
        fail=$((fail + 1))
    fi
}

run_neg_plain() {
    local base=$1
    local pattern=$2
    local hint=${3:-}
    case_paths "$base"
    if $JRUST build "$CASE_SRC" >"$TMP/out" 2>&1; then
        echo "FAIL $base：本应报错却成功了"
        fail=$((fail + 1))
        return
    fi
    if ! grep -q -- "$pattern" "$TMP/out"; then
        echo "FAIL $base：缺少错误信息（$pattern）"
        cat "$TMP/out"
        fail=$((fail + 1))
        return
    fi
    # 部分解析错误没有「提示:」行，此时第 3 参数留空即可跳过。
    if [ -n "$hint" ] && { ! grep -q "提示:" "$TMP/out" || ! grep -q -- "$hint" "$TMP/out"; }; then
        echo "FAIL $base：提示行不符合预期（$hint）"
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
    case_paths "$base"
    if ! $JRUST build "$CASE_SRC" >/dev/null 2>&1; then
        echo "FAIL $base：转译失败"
        fail=$((fail + 1))
        return
    fi
    if grep -q '|x| x \* 2' "$CASE_RS" \
        && grep -q '|x| x + 1' "$CASE_RS" \
        && grep -q '|a: f64, b: f64| a + b' "$CASE_RS"; then
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
    case_paths "$base"
    if ! $JRUST build "$CASE_SRC" >/dev/null 2>"$TMP/err"; then
        echo "FAIL $base：转译失败"
        cat "$TMP/err"
        fail=$((fail + 1))
        return
    fi
    if grep -q -- "$pattern" "$CASE_RS"; then
        echo "PASS $base"
        pass=$((pass + 1))
    else
        echo "FAIL $base：产物里没有预期写法（$pattern）"
        fail=$((fail + 1))
    fi
}

# 产物形态的「不应出现」断言：转译必须成功，且产物里不能有指定写法（锁定已知限制）。
run_build_lacks() {
    local base=$1
    local pattern=$2
    case_paths "$base"
    if ! $JRUST build "$CASE_SRC" >/dev/null 2>"$TMP/err"; then
        echo "FAIL $base：转译失败"
        cat "$TMP/err"
        fail=$((fail + 1))
        return
    fi
    if grep -q -- "$pattern" "$CASE_RS"; then
        echo "FAIL $base：产物里不应出现（$pattern）"
        fail=$((fail + 1))
    else
        echo "PASS $base"
        pass=$((pass + 1))
    fi
}

# warning 级诊断：转译成功（退出 0），但会提示未知签名（JR4101），实参按只读借用生成。
run_warn() {
    local base=$1
    local pattern=$2
    case_paths "$base"
    if ! $JRUST build "$CASE_SRC" >"$TMP/out" 2>&1; then
        echo "FAIL $base：转译失败"
        cat "$TMP/out"
        fail=$((fail + 1))
        return
    fi
    if ! grep -q 'warning\[JR4101\]' "$TMP/out" || ! grep -q '@mut' "$TMP/out"; then
        echo "FAIL $base：缺少 warning[JR4101] 或 @mut 提示"
        cat "$TMP/out"
        fail=$((fail + 1))
        return
    fi
    if ! grep -q -- "$pattern" "$CASE_RS"; then
        echo "FAIL $base：产物里没有预期写法（$pattern）"
        fail=$((fail + 1))
        return
    fi
    echo "PASS $base"
    pass=$((pass + 1))
}

# `jrust check`：只做检查、不写产物；ok 期望退出 0，fail 期望非 0。
run_check() {
    local base=$1
    local expect=$2
    case_paths "$base"
    if [ "$expect" = ok ]; then
        if $JRUST check "$CASE_SRC" >"$TMP/out" 2>&1; then
            echo "PASS check:$base"
            pass=$((pass + 1))
        else
            echo "FAIL check:$base：本应通过"
            cat "$TMP/out"
            fail=$((fail + 1))
        fi
    else
        if $JRUST check "$CASE_SRC" >"$TMP/out" 2>&1; then
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
    # Stage C：跨文件方法的借用形参 / 调用点按 `&T` 传参。
    if ! grep -q 'pub fn greet(&self, name: &str) -> String' "$work/app/greet.rs"; then
        echo "FAIL multifile：跨文件方法没有生成借用形参"
        grep -n 'fn greet' "$work/app/greet.rs"
        fail=$((fail + 1))
        return
    fi
    if ! grep -q 'g.greet(&who)' "$work/main.rs"; then
        echo "FAIL multifile：跨文件调用点没有按借用传参"
        grep -n 'greet' "$work/main.rs"
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
    if [ "$(cat "$TMP/out")" = "$(printf 'Hello, jrust\nHi, world')" ]; then
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

# 缺口 4：不同 package 下的同名类共享签名键，曾让不动点 Ref ↔ Mut 振荡（analyze 死循环）。
# 同时守住「构建能结束（不挂起）」和「产物能编译运行」。
run_multifile_same() {
    local work="$TMP/multifile_same"
    rm -rf "$work"
    cp -r tests/multifile_same "$work"
    if ! timeout 30 $JRUST build "$work" >"$TMP/out" 2>&1; then
        echo "FAIL multifile_same：转译失败或超时"
        cat "$TMP/out"
        fail=$((fail + 1))
        return
    fi
    if ! rustc --edition 2021 -A warnings "$work/main.rs" -o "$TMP/ms" 2>"$TMP/err"; then
        echo "FAIL multifile_same：生成的 Rust 编译失败"
        cat "$TMP/err"
        fail=$((fail + 1))
        return
    fi
    "$TMP/ms" >"$TMP/out"
    if [ "$(cat "$TMP/out")" = "x!" ]; then
        echo "PASS multifile_same"
        pass=$((pass + 1))
    else
        echo "FAIL multifile_same：输出是 $(cat "$TMP/out")"
        fail=$((fail + 1))
    fi
}

# 缺口 3：外部 crate 的构造器没有签名信息，实参按 owned 生成（不能是 `&s`）。
run_deps_ctor() {
    local work="$TMP/deps_ctor"
    rm -rf "$work"
    cp -r tests/deps_ok "$work"
    if ! $JRUST build "$work/ctor.jrust" >"$TMP/out" 2>&1; then
        echo "FAIL deps_ctor：转译失败"
        cat "$TMP/out"
        fail=$((fail + 1))
        return
    fi
    if grep -q 'Value::new(s)' "$work/ctor.rs" && ! grep -q 'Value::new(&s)' "$work/ctor.rs"; then
        echo "PASS deps_ctor"
        pass=$((pass + 1))
    else
        echo "FAIL deps_ctor：外部构造器实参不是 owned"
        grep -n 'Value::new' "$work/ctor.rs"
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
    case_paths "$base"
    if $JRUST verify "$CASE_SRC" >"$TMP/out" 2>&1; then
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

# JR-X3：raw 内容原样内联、不参与 jrust 诊断，verify 报告里单列块数与行数。
run_verify_raw() {
    local out
    if ! out=$($JRUST verify tests/conformance/L1/ok_raw/main.jrust 2>&1); then
        echo "FAIL verify_raw：ok_raw 应通过"
        echo "$out"
        fail=$((fail + 1))
        return
    fi
    if echo "$out" | grep -Eq '^  raw: [0-9]+ 块 / [0-9]+ 行（原样内联，不计入 jrust 诊断覆盖率）$'; then
        echo "PASS verify_raw"
        pass=$((pass + 1))
    else
        echo "FAIL verify_raw：报告缺少 raw 统计行"
        echo "$out"
        fail=$((fail + 1))
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

# Stage E / JR-C11 / JR-C15：`spawn` 捕获 clone 后的产物用 cargo + tokio 真实编译运行。
# spawn 是并发，输出顺序不确定，因此只断言「同一行出现的次数」而不是逐字节顺序。
run_spawn_capture() {
    local work="$TMP/spawn_capture"
    rm -rf "$work"
    cp -r tests/spawn_capture "$work"
    if ! $JRUST build "$work/src/main.jrust" >"$TMP/out" 2>&1; then
        echo "FAIL spawn_capture：转译失败"
        cat "$TMP/out"
        fail=$((fail + 1))
        return
    fi
    if ! cargo build --quiet --manifest-path "$work/Cargo.toml" >"$TMP/out" 2>&1; then
        echo "FAIL spawn_capture：cargo 编译失败"
        cat "$TMP/out"
        fail=$((fail + 1))
        return
    fi
    cargo run --quiet --manifest-path "$work/Cargo.toml" >"$TMP/out" 2>&1
    # 期望：闭包内 `1`（xs.size()）+ 外层 `hello` + 外层 `1`。
    if [ "$(grep -c '^hello$' "$TMP/out")" = "1" ] && [ "$(grep -c '^1$' "$TMP/out")" = "2" ]; then
        echo "PASS spawn_capture"
        pass=$((pass + 1))
    else
        echo "FAIL spawn_capture：输出不符合预期"
        cat "$TMP/out"
        fail=$((fail + 1))
    fi
}

# L2（SPEC §15）：多文件工程 + 一个外部 crate（serde_json）。
# 验收标准是「`cargo build` 通过且行为与手写版本一致」，所以这里走 cargo 真实编译，
# 再把 jrust 产物（bin `generated`）与手写参考实现（bin `reference`）的 stdout 逐字节对拍。
# 另附三条产物断言：`package app` → `pub mod app;`、外部 import → `use serde_json::Value;`、
# 本地 import → `use crate::app::Config;`（JR-S4/M3 的 crate 前缀）。
run_l2() {
    local work="$TMP/l2"
    rm -rf "$work"
    cp -r tests/l2 "$work"
    if ! $JRUST build "$work/src" >"$TMP/out" 2>&1; then
        echo "FAIL l2：转译失败"
        cat "$TMP/out"
        fail=$((fail + 1))
        return
    fi
    if ! grep -q '^pub mod app;$' "$work/src/main.rs"; then
        echo "FAIL l2：crate 根缺少 pub mod app;"
        cat "$work/src/main.rs"
        fail=$((fail + 1))
        return
    fi
    if ! grep -q '^use serde_json::Value;$' "$work/src/main.rs"; then
        echo "FAIL l2：外部 crate 的 import 没有生成 use"
        cat "$work/src/main.rs"
        fail=$((fail + 1))
        return
    fi
    if ! grep -q '^use crate::app::Config;$' "$work/src/main.rs"; then
        echo "FAIL l2：本地模块的 import 没有加 crate:: 前缀"
        cat "$work/src/main.rs"
        fail=$((fail + 1))
        return
    fi
    # 先离线（本地缓存通常已有 serde_json）；CI 首次运行没有缓存，回退联网拉取一次。
    if ! cargo build --quiet --offline --manifest-path "$work/Cargo.toml" >"$TMP/out" 2>&1; then
        if ! cargo build --quiet --manifest-path "$work/Cargo.toml" >>"$TMP/out" 2>&1; then
            echo "FAIL l2：cargo 编译失败"
            cat "$TMP/out"
            fail=$((fail + 1))
            return
        fi
    fi
    "$work/target/debug/generated" >"$TMP/gen_out" 2>&1
    if ! diff -u "$work/expected.txt" "$TMP/gen_out" >"$TMP/diff"; then
        echo "FAIL l2：jrust 产物输出与期望不一致"
        cat "$TMP/diff"
        fail=$((fail + 1))
        return
    fi
    # 手写参考实现也必须产出同样的输出（否则期望值本身有问题）。
    "$work/target/debug/reference" >"$TMP/ref_out" 2>&1
    if ! diff -u "$TMP/gen_out" "$TMP/ref_out" >"$TMP/diff"; then
        echo "FAIL l2：产物与手写参考实现行为不一致"
        cat "$TMP/diff"
        fail=$((fail + 1))
        return
    fi
    echo "PASS l2"
    pass=$((pass + 1))
}

# L2 的负向面：目录工程里的外部 crate 未在 Cargo.toml 声明 → JR5001。
run_l2_cross() {
    # 跨文件矩阵：构造器 / 实例方法 / 静态方法 / 可空返回 / enum / 接口作类型 /
    # 字符串拼接，全部跨文件。v1.24 之前类型表只喂当前文件，这七条全编译失败。
    local work="$TMP/l2_cross"
    rm -rf "$work"
    cp -r tests/l2_cross "$work"
    rm -rf "$work/target"
    if ! $JRUST build "$work" >"$TMP/out" 2>&1; then
        echo "FAIL l2_cross：转译失败"
        cat "$TMP/out"
        fail=$((fail + 1))
        return
    fi
    if ! cargo build --quiet --offline --manifest-path "$work/Cargo.toml" >"$TMP/out" 2>&1; then
        echo "FAIL l2_cross：cargo 编译失败（跨文件类型信息丢失？）"
        cat "$TMP/out"
        fail=$((fail + 1))
        return
    fi
    "$work/target/debug/generated" >"$TMP/gen_out" 2>&1
    "$work/target/debug/reference" >"$TMP/ref_out" 2>&1
    if ! diff -u "$TMP/ref_out" "$TMP/gen_out" >"$TMP/diff"; then
        echo "FAIL l2_cross：产物与手写参考实现行为不一致"
        cat "$TMP/diff"
        fail=$((fail + 1))
        return
    fi
    echo "PASS l2_cross"
    pass=$((pass + 1))
}

# 跨文件构造器实参必须补 `.to_string()`（P0 第 1 条）。
run_l2_cross_ctor() {
    local work="$TMP/l2_ctor"
    rm -rf "$work"
    cp -r tests/l2_cross "$work"
    rm -rf "$work/target"
    $JRUST build "$work" >"$TMP/out" 2>&1 || { echo "FAIL l2_ctor：转译失败"; cat "$TMP/out"; fail=$((fail + 1)); return; }
    if ! grep -q 'Point::new("api".to_string(), 3, 4)' "$work/main.rs"; then
        echo "FAIL l2_ctor：跨文件构造器实参没有补 .to_string()"
        grep -n "Point::new" "$work/main.rs"
        fail=$((fail + 1))
        return
    fi
    if ! grep -q 'Point::echo("hi".to_string())' "$work/main.rs"; then
        echo "FAIL l2_ctor：跨文件静态方法实参没有补 .to_string()"
        fail=$((fail + 1))
        return
    fi
    echo "PASS l2_ctor"
    pass=$((pass + 1))
}

# 跨文件可空返回不能包成 Some(...)（P0 第 7 条）。
run_l2_cross_nullable() {
    local work="$TMP/l2_null"
    rm -rf "$work"
    cp -r tests/l2_cross "$work"
    rm -rf "$work/target"
    $JRUST build "$work" >"$TMP/out" 2>&1 || { echo "FAIL l2_null：转译失败"; cat "$TMP/out"; fail=$((fail + 1)); return; }
    if grep -q 'Some(o.nickname())' "$work/main.rs"; then
        echo "FAIL l2_null：跨文件可空返回被包成了 Some(...)（双重 Option）"
        fail=$((fail + 1))
        return
    fi
    echo "PASS l2_null"
    pass=$((pass + 1))
}

# 跨文件接口作类型要包 Box<dyn>（P0 第 5 条）。
run_l2_cross_iface() {
    local work="$TMP/l2_iface"
    rm -rf "$work"
    cp -r tests/l2_cross "$work"
    rm -rf "$work/target"
    $JRUST build "$work" >"$TMP/out" 2>&1 || { echo "FAIL l2_iface：转译失败"; cat "$TMP/out"; fail=$((fail + 1)); return; }
    if ! grep -q 'let s: Box<dyn Shape>' "$work/main.rs"; then
        echo "FAIL l2_iface：跨文件接口作类型没有包 Box<dyn>"
        grep -n "Shape" "$work/main.rs"
        fail=$((fail + 1))
        return
    fi
    echo "PASS l2_iface"
    pass=$((pass + 1))
}

# L2 的负向面：目录工程里的外部 crate 未在 Cargo.toml 声明 → JR5001。
run_l2_missing_dep() {
    local work="$TMP/l2_nodep"
    rm -rf "$work"
    cp -r tests/l2 "$work"
    sed -i '/^serde_json/d' "$work/Cargo.toml"
    if $JRUST build "$work/src" >"$TMP/out" 2>&1; then
        echo "FAIL l2_nodep：本应报错却成功了"
        fail=$((fail + 1))
        return
    fi
    if grep -q 'error\[JR5001\]' "$TMP/out" && grep -q 'serde_json' "$TMP/out"; then
        echo "PASS l2_nodep"
        pass=$((pass + 1))
    else
        echo "FAIL l2_nodep：缺少 JR5001 / serde_json"
        cat "$TMP/out"
        fail=$((fail + 1))
    fi
}

# L2 续：`jrust verify <目录>` 自身的验收（此前这一入口零覆盖，五个坑都是靠手工试出来的）。
# 三条：① 正向通过；② 编译错误能映射回 .jrust 行；③ 搬不走的 manifest 给明确诊断。
# ②③ 显式注入 CARGO_TERM_COLOR=always —— 复现 Agent/CI 环境，颜色一旦漏禁，
# 映射会静默退化成「未能定位到 .jrust 行」。
run_verify_tree() {
    local work="$TMP/vtree"
    rm -rf "$work"
    cp -r tests/l2 "$work"
    rm -rf "$work/target" "$work/src/app.rs" "$work/src/main.rs"
    if ! CARGO_TERM_COLOR=always $JRUST verify "$work/src" >"$TMP/out" 2>&1; then
        echo "FAIL vtree：目录校验本应通过"
        cat "$TMP/out"
        fail=$((fail + 1))
        return
    fi
    # 搬不走的 bin（l2 的手写 ref/reference.rs）要明确提示，不能静默丢弃。
    if ! grep -q '跳过 1 个 \[\[bin\]\]' "$TMP/out" || ! grep -q 'ref/reference.rs' "$TMP/out"; then
        echo "FAIL vtree：缺少「跳过 ref/reference.rs」的提示"
        cat "$TMP/out"
        fail=$((fail + 1))
        return
    fi
    echo "PASS vtree"
    pass=$((pass + 1))
}

run_verify_tree_bad() {
    local work="$TMP/vtree_bad"
    rm -rf "$work"
    cp -r tests/l2 "$work"
    rm -rf "$work/target" "$work/src/app.rs" "$work/src/main.rs"
    # 把 raws 里的调用改成 String 上不存在的方法 → rustc E0599 落在 raw 块内。
    sed -i 's/v\["port"\].as_i64()/name.as_i64()/' "$work/src/main.jrust"
    if CARGO_TERM_COLOR=always $JRUST verify "$work/src" >"$TMP/out" 2>&1; then
        echo "FAIL vtree_bad：本应报错却成功了"
        cat "$TMP/out"
        fail=$((fail + 1))
        return
    fi
    if grep -q '未能定位' "$TMP/out"; then
        echo "FAIL vtree_bad：错误没有映射回 .jrust 行（cargo 颜色没禁用？）"
        cat "$TMP/out"
        fail=$((fail + 1))
        return
    fi
    if ! grep -qE 'main\.jrust:[0-9]+ - rustc error\[E[0-9]+\]' "$TMP/out"; then
        echo "FAIL vtree_bad：缺少映射到 main.jrust 的错误行"
        cat "$TMP/out"
        fail=$((fail + 1))
        return
    fi
    echo "PASS vtree_bad"
    pass=$((pass + 1))
}

run_verify_tree_manifest() {
    local work="$TMP/vtree_manifest"
    rm -rf "$work"
    cp -r tests/l2 "$work"
    rm -rf "$work/target" "$work/src/app.rs" "$work/src/main.rs"
    # build 脚本没法搬进临时目录：必须给出明确诊断，而不是 cargo 的 "couldn't read build.rs"。
    sed -i 's/^edition = "2021"$/edition = "2021"\nbuild = "build.rs"/' "$work/Cargo.toml"
    if $JRUST verify "$work/src" >"$TMP/out" 2>&1; then
        echo "FAIL vtree_manifest：本应报错却成功了"
        cat "$TMP/out"
        fail=$((fail + 1))
        return
    fi
    if ! grep -q 'build 脚本' "$TMP/out" || grep -q '未能定位' "$TMP/out"; then
        echo "FAIL vtree_manifest：缺少关于 build 脚本的明确诊断"
        cat "$TMP/out"
        fail=$((fail + 1))
        return
    fi
    echo "PASS vtree_manifest"
    pass=$((pass + 1))
}

# 依赖段的三种写法必须被**同一个**解析器识别（v1.24 之前两个函数口径不一致：
# `[dependencies.serde_json]` 校验认、重建丢 → 「校验通过但 cargo 说没这个 crate」）。
run_verify_tree_dep_forms() {
    local work="$TMP/vtree_depforms"
    rm -rf "$work"
    cp -r tests/l2 "$work"
    rm -rf "$work/target" "$work/src/app.rs" "$work/src/main.rs"
    # 子表形式：`[dependencies.serde_json]` + version 子键。
    cat > "$work/Cargo.toml" <<'EOF'
[package]
name = "l2"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "generated"
path = "src/main.rs"

[dependencies.serde_json]
version = "1"
EOF
    if ! $JRUST verify "$work/src" >"$TMP/out" 2>&1; then
        echo "FAIL vtree_depforms：子表形式 [dependencies.serde_json] 本应通过"
        cat "$TMP/out"
        fail=$((fail + 1))
        return
    fi
    # target 限定的子表形式也要认（否则误报 JR5001）。
    cat > "$work/Cargo.toml" <<'EOF'
[package]
name = "l2"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "generated"
path = "src/main.rs"

[target.'cfg(unix)'.dependencies.serde_json]
version = "1"
EOF
    if ! $JRUST verify "$work/src" >"$TMP/out" 2>&1; then
        echo "FAIL vtree_depforms：target 子表形式本应通过"
        cat "$TMP/out"
        fail=$((fail + 1))
        return
    fi
    # 反向：真的没声明时必须报 JR5001（而不是静默走到 cargo）。
    cat > "$work/Cargo.toml" <<'EOF'
[package]
name = "l2"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "generated"
path = "src/main.rs"
EOF
    if $JRUST verify "$work/src" >"$TMP/out" 2>&1; then
        echo "FAIL vtree_depforms：缺依赖本应报错"
        fail=$((fail + 1))
        return
    fi
    if ! grep -q 'JR5001' "$TMP/out"; then
        echo "FAIL vtree_depforms：缺依赖没有报 JR5001"
        cat "$TMP/out"
        fail=$((fail + 1))
        return
    fi
    echo "PASS vtree_depforms"
    pass=$((pass + 1))
}

# L2 续：无 main 的多文件工程按 lib crate 校验（此前报「没有含 main 的 crate 根」，与
# 单文件模式支持 `--crate-type lib` 的行为不一致）。负向用例证明它真的编了 lib 而不是空跑。
run_verify_tree_lib() {
    local work="$TMP/vtree_lib"
    rm -rf "$work"
    mkdir -p "$work"
    cat > "$work/Cargo.toml" <<'EOF'
[package]
name = "vtree-lib"
version = "0.1.0"
edition = "2021"
EOF
    cat > "$work/shape.jrust" <<'EOF'
package shape;

class Point {
    public int x;
    public Point(int x) { this.x = x; }
}
EOF
    if ! $JRUST verify "$work" >"$TMP/out" 2>&1; then
        echo "FAIL vtree_lib：lib-only 工程本应通过"
        cat "$TMP/out"
        fail=$((fail + 1))
        return
    fi
    # 负向：raw 里调用不存在的方法，错误必须映射回 shape.jrust 的行。
    cat > "$work/shape.jrust" <<'EOF'
package shape;

class Point {
    public int x;
    public Point(int x) { this.x = x; }
    public int bad() {
        return raw { self.nope() };
    }
}
EOF
    if $JRUST verify "$work" >"$TMP/out" 2>&1; then
        echo "FAIL vtree_lib_bad：本应报错却成功了（lib 没真正编译？）"
        fail=$((fail + 1))
        return
    fi
    if ! grep -qE 'shape\.jrust:[0-9]+ - rustc error\[E[0-9]+\]' "$TMP/out"; then
        echo "FAIL vtree_lib_bad：错误没有映射回 shape.jrust 行"
        cat "$TMP/out"
        fail=$((fail + 1))
        return
    fi
    # 正向 2：lib 根文件里可以直接放 `pub struct`（无 package 的根文件就是 lib 根）。
    # v1.24 之前 root_out 固定成自动生成的 lib.rs，用户文件只能是子模块，
    # 写 `lib.jrust` 会 `circular modules`、不写 package 会 JR3029 —— 两条路都堵死。
    rm -f "$work/shape.jrust"
    cat > "$work/lib.jrust" <<'EOF'
public class Root {
    public static int v() { return 1; }
}
EOF
    if ! $JRUST verify "$work" >"$TMP/out" 2>&1; then
        echo "FAIL vtree_lib_root：lib 根文件里放内容本应通过"
        cat "$TMP/out"
        fail=$((fail + 1))
        return
    fi
    echo "PASS vtree_lib"
    pass=$((pass + 1))
}

# 产物开 `-D warnings` 也必须干净：raw 外层括号会触发 unused_parens，
# 生成头的 allow 列表必须覆盖它（此前只有 4 项，用户开 -D warnings 就编不过）。
run_strict_warnings() {
    # 用带驼峰方法名的用例：`ok_raw` 恰好没有驼峰方法，所以这个坑藏了很久
    # （`non_snake_case` 在 `-D warnings` 下是错误）。两个用例一起跑，覆盖
    # 「驼峰命名」与「先声明后赋值」两类生成形态噪声。
    for base in ok_raw ok_nullable; do
        case_paths "$base"
        if ! $JRUST build "$CASE_SRC" >"$TMP/out" 2>&1; then
            echo "FAIL strict_warnings($base)：转译失败"
            cat "$TMP/out"
            fail=$((fail + 1))
            return
        fi
        if ! rustc --edition 2021 -D warnings "$CASE_RS" -o "$TMP/strict" >"$TMP/out" 2>&1; then
            echo "FAIL strict_warnings($base)：产物在 -D warnings 下编译失败"
            cat "$TMP/out"
            fail=$((fail + 1))
            return
        fi
    done
    # 13 个例程里驼峰最密集的几个：这条断言直接对应 JR-G9 的口径。
    for name in Nullable SharedState MoveEdges; do
        local rs="examples/$name/main.rs"
        [ -f "$rs" ] || continue
        if ! rustc --edition 2021 -D warnings --crate-type bin "$rs" -o "$TMP/strict" >"$TMP/out" 2>&1; then
            echo "FAIL strict_warnings(examples/$name)：例程产物在 -D warnings 下编译失败"
            cat "$TMP/out"
            fail=$((fail + 1))
            return
        fi
    done
    echo "PASS strict_warnings"
    pass=$((pass + 1))
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
run_ok ok_raw_multiline
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
run_ok ok_borrow_param
run_ok ok_borrow_string
run_ok ok_list_get_owned
run_ok ok_keyword_escape
run_ok ok_borrow_list
run_ok ok_getter_ref
run_ok ok_pick_lifetime
run_ok ok_override_ref
# 缺口 6：闭包按引用捕获 this 字段与局部变量，外层变量仍可用（rustc 真实编译运行）。
run_ok ok_closure_capture
run_closure
run_build_has ok_spawn 'tokio::spawn(async move {'
# Stage E：捕获的局部变量在 spawn 前 clone（Copy 标量不 clone），外层仍可用。
run_build_has ok_spawn_capture 'let __jr_sp0_s = s.clone();'
run_build_has ok_spawn_capture 'let __jr_sp0_xs = xs.clone();'
run_build_has ok_spawn_capture 'let s = __jr_sp0_s;'
run_build_has ok_async_main '#\[tokio::main\]'
run_build_has ok_async_main 'async fn main'
run_build_has ok_illegal_arg 'panic!'
run_build_has ok_custom_exception 'pub struct AppError'
run_build_has ok_custom_exception 'impl std::error::Error for AppError'
run_build_has ok_mut_receiver 'fn inc(&mut self)'
run_build_has ok_mut_receiver 'fn get(&self)'
run_build_has ok_borrow_param 'xs: &mut Vec<i32>'
run_build_has ok_borrow_param 'c: &mut Counter'
run_build_has ok_borrow_param 'xs: &\[i32\]'
run_build_has ok_borrow_string 'who: &str'
run_build_has ok_borrow_list 'xs: &\[String\]'
run_build_has ok_getter_ref 'fn name(&self) -> &str'
run_build_has ok_getter_ref 'fn age(&self) -> i32'
run_build_has ok_pick_lifetime "fn pick<'a>(&self, x: &'a str, y: &str) -> &'a str"
run_build_has ok_pick_lifetime "fn choose<'a>(&self, x: &'a str, y: &'a str) -> &'a str"
run_build_has ok_pick_lifetime 'fn tag(&self) -> &str'
# Stage D：`@ref` / `@mut` 强制覆盖推断（`@mut` 把只读用法也写成 `&mut Vec`）。
run_build_has ok_override_ref 'fn touch(&self, xs: &mut Vec<i32>)'
run_build_has ok_override_ref 'fn count(&self, xs: &\[i32\]) -> i32'
run_build_has ok_override_ref 'u.touch(&mut xs)'
# `@mut String` 按 D2 落 owned：签名与调用点都必须是 String，不能生成 `&mut String`。
run_build_has ok_override_ref 'fn note(&self, s: String)'
run_build_has ok_override_ref 'u.note(note)'
run_warn ok_ext_unknown 'w.write(&text)'
# 缺口 1：spawn 不能捕获 this（JR3023），替代路径是字段先复制到局部变量。
run_build_has ok_spawn_this_copy 'let t: String = self.tag.clone();'
run_build_has ok_spawn_this_copy 'let __jr_sp0_t = t.clone();'
# 缺口 2：raw 块内的自由变量不参与 spawn 捕获克隆（已知限制，锁定现状）。
run_build_has ok_spawn_raw 'println!("{}", s);'
run_build_lacks ok_spawn_raw '__jr_sp0_s'
# 缺口 3：外部 crate 构造器的实参按 owned 生成，不做借用推断。
run_deps_ctor

run_neg neg_try_without_throws JR3018 "给当前方法签名加" 4
run_neg neg_switch_int JR3019 "int / String" 4
run_neg neg_switch_incomplete JR3019 "补上缺少的 case" 9
run_neg neg_fallthrough JR3020 "给该分支补" 5
run_neg neg_throw_without_throws JR3021 "在方法签名后加" 3
run_neg neg_throw_undeclared JR3021 "在方法签名里声明" 3
run_neg neg_null_assign JR3005 "改用一个非空值" 3
run_neg neg_nullable_method JR3006 "先判空再取出来" 16
run_neg neg_cfor_step JR2003 "步长不为 1" 3
run_neg neg_cfor_dir JR2003 "方向不一致" 3
run_neg neg_newarray_type JR3014 "只支持标量 / String" 3
run_neg neg_wildcard JR3010 "trait object" 6
run_neg neg_mutable_static JR3013 "可变全局" 3
run_neg neg_await_sync JR3022 "async" 4
run_neg neg_spawn_this JR3023 "this" 13
run_neg neg_iface_static_no_body JR3024 "必须有方法体" 3
run_neg neg_iter_unknown JR3017 "白名单" 6
run_neg neg_sync_target JR3026 "this.f" 5
run_neg neg_labeled JR2004 "不支持带标签" 4
run_neg neg_raw_nested JR3027 "不能嵌套" 4
run_neg neg_mutable_set JR3028 "只适用于 List" 6
# v1.37 定案：try/catch/finally 永久不支持，解析期用 JR3031 给出替代方案。
run_neg neg_try JR3031 "不支持 try / catch / finally" 3
run_neg neg_return_this JR4003 "静态工厂" 11
run_neg neg_conflict JR4001 "所有权已经移走" 18
run_neg neg_borrow_field JR3011 "无法统一" 18
run_neg neg_override_conflict JR4001 "需要它的所有权" 9
# Stage D：JR4001 的另外五类触发（此前只有「存字段」一类有用例）。
run_neg neg_override_ref_mutates JR4001 "会修改它" 3
run_neg neg_override_scalar JR4001 "按值复制" 3
run_neg neg_override_generic JR4001 "泛型类型参数一律按值" 9
run_neg neg_override_static JR4001 "静态方法没有" 3
run_neg neg_override_iface JR4001 "实现签名冲突" 8
# parser 层注解错误（无诊断码）：位置、参数、互斥。
run_neg_plain neg_override_misplaced "只能写在方法形参上" "写在方法签名或接口方法声明的形参前"
run_neg_plain neg_override_args "不接受参数" "括号里什么都不用填"
run_neg_plain neg_override_both "只能出现一个" "两者互斥"
# 缺口 5：@ref / @mut 不适用于构造器形参、枚举载荷与返回值位置。
run_neg_plain neg_ref_ctor "只能写在方法形参上" "写在方法签名或接口方法声明的形参前"
run_neg_plain neg_ref_enum "只能写在方法形参上" "写在方法签名或接口方法声明的形参前"
run_neg_plain neg_ref_return "这里期望一个类型"

run_check ok_shared ok
run_check neg_labeled fail
run_check_bulk
run_multifile
run_multifile_neg
# 缺口 4：跨 package 同名类的签名键冲突（曾导致 analyze 死循环）。
run_multifile_same
run_deps tests/deps_bad/bad.jrust fail 'serde_json'
run_deps tests/deps_ok/ok.jrust ok
run_deps tests/deps_async/src/main.jrust fail 'tokio'
run_verify verify_ok ok
run_verify verify_raw_error fail 'raw 块内'
run_verify_raw
run_fmt
run_spawn_capture
# SPEC §15 L2：多文件 + 外部 crate，cargo 编译 + 与手写实现行为对拍。
run_l2
run_l2_cross
run_l2_cross_ctor
run_l2_cross_nullable
run_l2_cross_iface
run_l2_missing_dep
# `jrust verify <目录>` 自身：正向 / 错误映射 / manifest 诊断。
run_verify_tree
run_verify_tree_bad
run_verify_tree_manifest
run_verify_tree_dep_forms
run_verify_tree_lib
run_strict_warnings

echo "L1 用例：$pass 通过，$fail 失败"
[ "$fail" -eq 0 ]
