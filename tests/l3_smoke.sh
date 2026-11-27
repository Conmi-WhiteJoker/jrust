#!/usr/bin/env bash
# L3 验收（SPEC §15）：async 小服务——axum + 共享状态 + 优雅退出。
#   1) 转译 + 编译；2) 端点 smoke（/health、/hit 递增）；3) ab 压测 ≥1k QPS、0 失败；
#   4) SIGINT 后走优雅退出分支、退出码 0。
set -euo pipefail
cd "$(dirname "$0")/.."

PORT=${L3_PORT:-18080}
BASE="http://127.0.0.1:$PORT"
BIN=tests/l3/target/debug/l3
LOG=$(mktemp)
fail=0

echo "== 1/4 转译并编译 L3 服务 =="
./target/debug/jrust build tests/l3/src >/dev/null
cargo build --quiet --manifest-path tests/l3/Cargo.toml

echo "== 2/4 启动服务并检查端点 =="
"$BIN" >"$LOG" 2>&1 &
pid=$!
cleanup() {
    kill -INT "$pid" 2>/dev/null || true
    wait "$pid" 2>/dev/null || true
}
trap cleanup EXIT

ready=0
for _ in $(seq 1 100); do
    if curl -sf "$BASE/health" >/dev/null 2>&1; then
        ready=1
        break
    fi
    sleep 0.1
done
if [ "$ready" -ne 1 ]; then
    echo "FAIL L3：服务 10s 内没有就绪"
    cat "$LOG"
    exit 1
fi

health=$(curl -s "$BASE/health")
if [ "$health" = "ok" ]; then
    echo "PASS L3：/health = ok"
else
    echo "FAIL L3：/health 返回 $health"
    fail=1
fi

h1=$(curl -s "$BASE/hit")
h2=$(curl -s "$BASE/hit")
if [ "$h1" = "1" ] && [ "$h2" = "2" ]; then
    echo "PASS L3：共享状态 /hit 递增 1 → 2"
else
    echo "FAIL L3：/hit 返回 $h1, $h2（期望 1, 2）"
    fail=1
fi

echo "== 3/4 压测（ab -n 3000 -c 32 $BASE/health）=="
ab_out=$(ab -n 3000 -c 32 "$BASE/health" 2>/dev/null)
failed=$(echo "$ab_out" | awk '/^Failed requests:/ {print $3}')
qps=$(echo "$ab_out" | awk '/^Requests per second:/ {print $4}')
echo "  Failed requests: ${failed:-?}，Requests per second: ${qps:-?}"
[ "${failed:-1}" = "0" ] || {
    echo "FAIL L3：压测存在失败请求"
    fail=1
}
awk -v q="${qps:-0}" 'BEGIN { exit !(q >= 1000) }' || {
    echo "FAIL L3：QPS 低于 1000"
    fail=1
}

echo "== 4/4 优雅退出（SIGINT）=="
kill -INT "$pid"
set +e
wait "$pid"
code=$?
set -e
trap - EXIT
if [ "$code" -eq 0 ]; then
    echo "PASS L3：SIGINT 后退出码 0"
else
    echo "FAIL L3：退出码 $code"
    fail=1
fi
if grep -q "l3 service shutdown clean" "$LOG"; then
    echo "PASS L3：走到优雅退出分支"
else
    echo "FAIL L3：没有打印 shutdown clean"
    cat "$LOG"
    fail=1
fi

rm -f "$LOG"
if [ "$fail" -ne 0 ]; then
    echo "L3 失败。"
    exit 1
fi
echo "L3 通过：smoke + ${qps} QPS / 0 失败 + 优雅退出。"
