#!/usr/bin/env bash
# L4 验收（SPEC §15）：async 中型服务全量转译——多模块 + 外部 crate + 共享状态 + 后台任务。
#   1) 转译 jrust 工程并编译两个 bin（generated = jrust 产物 / reference = 手写 Rust）
#   2) 同一组请求打到两个服务：状态码 + body 逐字节比对（含 JR-G13 三条失败路径）
#   3) 后台任务：ticks 持续增长；业务计数 hits 两侧一致
#   4) 压测（ab）双方 0 失败、QPS ≥1000，且 generated ≥ 0.7 × reference
#   5) SIGINT 两侧退出码 0 + 打印 shutdown clean
set -euo pipefail
cd "$(dirname "$0")/.."

BASE=${L4_PORT:-18081}
REF=$((BASE + 1))
BASE_URL="http://127.0.0.1:$BASE"
REF_URL="http://127.0.0.1:$REF"
GEN_BIN=tests/l4/target/debug/l4_generated
REF_BIN=tests/l4/target/debug/l4_reference
TMP=$(mktemp -d)
LOG_GEN=$TMP/generated.log
LOG_REF=$TMP/reference.log
fail=0
gpid=""
rpid=""

cleanup() {
    [ -n "$gpid" ] && kill -INT "$gpid" 2>/dev/null || true
    [ -n "$rpid" ] && kill -INT "$rpid" 2>/dev/null || true
    [ -n "$gpid" ] && wait "$gpid" 2>/dev/null || true
    [ -n "$rpid" ] && wait "$rpid" 2>/dev/null || true
    rm -rf "$TMP"
}
trap cleanup EXIT

echo "== 1/5 转译并编译 L4（generated + reference）=="
./target/debug/jrust build tests/l4/src >/dev/null
cargo build --quiet --manifest-path tests/l4/Cargo.toml

echo "== 2/5 启动两个实现 =="
"$GEN_BIN" "$BASE" >"$LOG_GEN" 2>&1 &
gpid=$!
"$REF_BIN" "$REF" >"$LOG_REF" 2>&1 &
rpid=$!

wait_ready() { # $1=url $2=日志
    local i
    for i in $(seq 1 100); do
        if curl -sf "$1/health" >/dev/null 2>&1; then
            return 0
        fi
        sleep 0.1
    done
    echo "FAIL L4：$1 10s 内没有就绪"
    cat "$2"
    return 1
}
wait_ready "$BASE_URL" "$LOG_GEN" || fail=1
wait_ready "$REF_URL" "$LOG_REF" || fail=1
if [ "$fail" -ne 0 ]; then
    exit 1
fi
echo "PASS L4：generated 监听 $BASE，reference 监听 $REF"

probe() { # $1=port $2=method $3=path $4=data；body 写入 $TMP/body_<port>，输出状态码
    local port=$1 method=$2 path=$3 data=${4:-}
    if [ "$method" = "POST" ]; then
        curl -s -o "$TMP/body_$port" -w '%{http_code}' -X POST --data "$data" \
            "http://127.0.0.1:$port$path"
    else
        curl -s -o "$TMP/body_$port" -w '%{http_code}' "http://127.0.0.1:$port$path"
    fi
}

parity() { # $1=名称 $2=method $3=path $4=data；仅比对两侧一致性，返回 0/1
    local name=$1 method=$2 path=$3 data=${4:-}
    local gcode rcode gb rb
    gcode=$(probe "$BASE" "$method" "$path" "$data")
    cp "$TMP/body_$BASE" "$TMP/g_body"
    rcode=$(probe "$REF" "$method" "$path" "$data")
    cp "$TMP/body_$REF" "$TMP/r_body"
    gb=$(cat "$TMP/g_body")
    rb=$(cat "$TMP/r_body")
    if [ "$gcode" != "$rcode" ] || [ "$gb" != "$rb" ]; then
        echo "FAIL L4：$name 两侧不一致（generated $gcode/$gb vs reference $rcode/$rb）"
        return 1
    fi
    PARITY_LAST="$gcode $gb"
    return 0
}

case_check() { # $1=名称 $2=method $3=path $4=data $5=期望状态码 $6=期望 body
    local name=$1
    if ! parity "$1" "$2" "$3" "${4:-}"; then
        fail=1
        return
    fi
    if [ "$PARITY_LAST" = "$5 $6" ]; then
        echo "PASS L4：$name → $PARITY_LAST"
    else
        echo "FAIL L4：$name 期望 $5 $6，实际 $PARITY_LAST"
        fail=1
    fi
}

echo "== 3/5 功能对拍（正常路径 + JR-G13 失败路径）=="
case_check "健康检查"       GET  /health            ""                  200 "ok"
case_check "登录成功"       POST /login             "password=s3cret"   200 "token:dev-1"
case_check "错密码 deny"    POST /login             "password=wrong"    401 "deny:bad-password"
case_check "已知设备"       GET  /devices/dev-1     ""                  200 "device:dev-1:online"
case_check "未知设备 deny"  GET  /devices/dev-9     ""                  404 "deny:unknown-device"
case_check "配置命中"       GET  /config/region     ""                  200 "cn-north-1"
case_check "键缺失 panic"   GET  /config/missing    ""                  500 "panic:missing-config-key"

echo "== 4/5 稳定性基线 + 后台任务 + 压测 =="
# 再跑 3 轮同样的 6 个业务请求：既看两侧是否始终一致，也让 hits 累加到可校验的值。
for round in 1 2 3; do
    for req in "POST /login password=s3cret" "POST /login password=wrong" \
               "GET /devices/dev-1 -" "GET /devices/dev-9 -" \
               "GET /config/region -" "GET /config/missing -"; do
        set -- $req
        if ! parity "第 $round 轮 $2 $3" "$1" "$2" "$3"; then
            fail=1
        fi
    done
done

ghits=$(curl -s "$BASE_URL/stats" | sed -n 's/^hits=\([0-9]*\) ticks=.*/\1/p')
rhits=$(curl -s "$REF_URL/stats" | sed -n 's/^hits=\([0-9]*\) ticks=.*/\1/p')
# 7 个 case_check 里 6 个是业务请求（health 不计），稳定性基线每轮 6 个，共 3 轮。
want_hits=$((6 + 3 * 6))
if [ "$ghits" = "$want_hits" ] && [ "$rhits" = "$want_hits" ]; then
    echo "PASS L4：业务计数 hits=$want_hits 两侧一致"
else
    echo "FAIL L4：hits 期望两侧 $want_hits，实际 generated=$ghits reference=$rhits"
    fail=1
fi

gt0=$(curl -s "$BASE_URL/stats" | sed -n 's/.*ticks=\([0-9]*\)$/\1/p')
rt0=$(curl -s "$REF_URL/stats" | sed -n 's/.*ticks=\([0-9]*\)$/\1/p')
sleep 0.5
gt1=$(curl -s "$BASE_URL/stats" | sed -n 's/.*ticks=\([0-9]*\)$/\1/p')
rt1=$(curl -s "$REF_URL/stats" | sed -n 's/.*ticks=\([0-9]*\)$/\1/p')
if [ "${gt1:-0}" -gt "${gt0:-0}" ] && [ "${rt1:-0}" -gt "${rt0:-0}" ]; then
    echo "PASS L4：后台任务 ticks 持续增长（generated $gt0→$gt1，reference $rt0→$rt1）"
else
    echo "FAIL L4：后台任务没有推进（generated $gt0→$gt1，reference $rt0→$rt1）"
    fail=1
fi

bench() { # $1=url：输出 "失败数 QPS"
    local out failed qps
    out=$(ab -n 2000 -c 20 "$1/health" 2>/dev/null)
    failed=$(echo "$out" | awk '/^Failed requests:/ {print $3}')
    qps=$(echo "$out" | awk '/^Requests per second:/ {print $4}')
    echo "${failed:-x} ${qps:-0}"
}
read -r gfail gqps <<< "$(bench "$BASE_URL")"
read -r rfail rqps <<< "$(bench "$REF_URL")"
echo "  generated: ${gfail:-?} 失败 / ${gqps:-?} QPS；reference: ${rfail:-?} 失败 / ${rqps:-?} QPS"

[ "$gfail" = "0" ] && [ "$rfail" = "0" ] || {
    echo "FAIL L4：压测存在失败请求（generated=$gfail reference=$rfail）"
    fail=1
}
awk -v q="${gqps:-0}" 'BEGIN { exit !(q >= 1000) }' || {
    echo "FAIL L4：generated QPS 低于 1000"
    fail=1
}
awk -v q="${rqps:-0}" 'BEGIN { exit !(q >= 1000) }' || {
    echo "FAIL L4：reference QPS 低于 1000"
    fail=1
}
awk -v g="${gqps:-0}" -v r="${rqps:-1}" 'BEGIN { exit !(g >= 0.7 * r) }' || {
    echo "FAIL L4：generated QPS 低于 reference 的 0.7 倍"
    fail=1
}

# 压测后健康检查仍正常：稳定性基线的最小判据。
after=$(curl -s "$BASE_URL/health")
if [ "$after" = "ok" ]; then
    echo "PASS L4：压测后 /health 仍为 ok"
else
    echo "FAIL L4：压测后 /health 返回 $after"
    fail=1
fi

echo "== 5/5 优雅退出（SIGINT）=="
kill -INT "$gpid"
kill -INT "$rpid"
set +e
wait "$gpid"
gcode=$?
wait "$rpid"
rcode=$?
set -e
trap - EXIT

if [ "$gcode" -eq 0 ] && [ "$rcode" -eq 0 ]; then
    echo "PASS L4：SIGINT 后两侧退出码 0"
else
    echo "FAIL L4：退出码 generated=$gcode reference=$rcode"
    fail=1
fi
if grep -q "l4 service shutdown clean" "$LOG_GEN" && grep -q "l4 service shutdown clean" "$LOG_REF"; then
    echo "PASS L4：两侧都走到优雅退出分支"
else
    echo "FAIL L4：没有打印 shutdown clean"
    echo "--- generated ---"; cat "$LOG_GEN"
    echo "--- reference ---"; cat "$LOG_REF"
    fail=1
fi

rm -rf "$TMP"
if [ "$fail" -ne 0 ]; then
    echo "L4 失败。"
    exit 1
fi
echo "L4 通过：功能对拍（含 3 条失败路径）+ 后台任务 + ${gqps}/${rqps} QPS + 优雅退出。"
