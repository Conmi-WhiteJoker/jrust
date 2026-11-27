#!/usr/bin/env bash
# 产物快照（V2 阶段 A 的硬门槛工具）。
#
# 用途：把当前转译器的产物指纹（sha256）记进 tests/snapshots/，之后任何重构
# 都可以用 `--check` 断言「产物逐字节未变」。阶段 A 的目标是纯结构改造
# （新增 types.rs、Env 换 TypeEnv），因此产物必须零漂移；漂移即回归。
#
# 用法：
#   ./tests/snapshot.sh           生成 / 更新快照（tests/snapshots/artifacts.txt）
#   ./tests/snapshot.sh --check   与已有快照比对，有差异则退出码 1
#
# 覆盖范围：所有能被 `jrust build` 单独转译的 .jrust 源。
#   - examples/*/main.jrust      单文件例程（15 个）
#   - tests/conformance/L1/*/main.jrust  一致性夹具（含负向：负向转译失败，只记「失败」）
#   - tests/multifile、tests/l2/src、tests/l3/src、tests/l4/src  多文件工程（目录模式）
#
# 设计取舍：只记 sha256，不存产物副本。产物本身可由 .jrust + 转译器重放，
# 存副本会让仓库膨胀且容易与源码不同步；指纹足以判定「有没有变」。
set -uo pipefail
cd "$(dirname "$0")/.."

JRUST=./target/debug/jrust
SNAP=tests/snapshots/artifacts.txt
CHECK=0
[ "${1:-}" = "--check" ] && CHECK=1

if [ ! -x "$JRUST" ]; then
    echo "错误：$JRUST 不存在，先 cargo build" >&2
    exit 2
fi

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

# 产物头部含源路径（`// 由 jrust 从 <path> 生成…`），临时目录每次不同会让指纹漂移。
# 把首行的路径归一成固定占位，只比较「除路径外的产物内容」。
normalize() {
    local f=$1
    sed -E '1s|^// 由 jrust 从 .* 生成。|// 由 jrust 从 <src> 生成。|' "$f"
}

# 单文件源：把源复制到临时目录再转译，产物就地取指纹（不污染工作区）。
# 注意 jrust build 没有 --out，产物固定落在源同目录同名 .rs。
snapshot_one() {
    local src=$1
    local name=$2
    local stem
    stem=$(basename "$src" .jrust)
    local work="$TMP/one_$(echo "$name" | tr '/' '_')"
    mkdir -p "$work"
    cp "$src" "$work/$stem.jrust"
    if "$JRUST" build "$work/$stem.jrust" >/dev/null 2>&1 && [ -f "$work/$stem.rs" ]; then
        printf '%s  %s\n' "$(normalize "$work/$stem.rs" | sha256sum | cut -d' ' -f1)" "$name"
    else
        # 负向夹具：转译本就应当失败。记录失败标记而不是跳过，
        # 这样「本该失败却成功了」也会被 --check 抓到。
        printf 'FAILED  %s\n' "$name"
    fi
}

# 目录工程：build 到临时目录，按相对路径逐个取指纹。
snapshot_dir() {
    local dir=$1
    local work="$TMP/dir_$(echo "$dir" | tr '/' '_')"
    mkdir -p "$work"
    cp -r "$dir"/. "$work"/ 2>/dev/null
    if ! "$JRUST" build "$work" >/dev/null 2>&1; then
        printf 'FAILED  %s\n' "$dir"
        return
    fi
    ( cd "$work" && find . -name '*.rs' -type f | sort | while read -r f; do
        printf '%s  %s/%s\n' "$(normalize "$f" | sha256sum | cut -d' ' -f1)" "$dir" "${f#./}"
    done )
}

{
    echo "# jrust 产物快照（sha256）。由 tests/snapshot.sh 生成，勿手改。"
    echo "# 格式：<sha256|FAILED>  <相对路径>"
    echo "# 更新：./tests/snapshot.sh    校验：./tests/snapshot.sh --check"

    for f in examples/*/main.jrust; do
        snapshot_one "$f" "$f"
    done
    for f in tests/conformance/L1/*/main.jrust; do
        snapshot_one "$f" "$f"
    done
    for d in tests/multifile tests/l2/src tests/l3/src tests/l4/src; do
        [ -d "$d" ] && snapshot_dir "$d"
    done
} > "$TMP/new.txt"

# 生成时统一排序（路径列），保证两次运行可比。
{ grep '^#' "$TMP/new.txt"; grep -v '^#' "$TMP/new.txt" | sort -k2; } > "$TMP/sorted.txt"

if [ "$CHECK" = 1 ]; then
    if [ ! -f "$SNAP" ]; then
        echo "错误：快照 $SNAP 不存在，先运行 ./tests/snapshot.sh 生成基线" >&2
        exit 2
    fi
    if diff -u "$SNAP" "$TMP/sorted.txt" > "$TMP/diff"; then
        echo "产物快照一致：$(grep -vc '^#' "$SNAP") 项逐字节未变。"
        exit 0
    fi
    echo "产物漂移（阶段 A 要求零漂移）："
    cat "$TMP/diff"
    exit 1
fi

mkdir -p tests/snapshots
cp "$TMP/sorted.txt" "$SNAP"
echo "已写入 $SNAP：$(grep -vc '^#' "$SNAP") 项。"
