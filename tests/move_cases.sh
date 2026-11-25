#!/usr/bin/env bash
# move 关键字的非法边界用例：每一种都应当转译失败（退出码 1），
# 并且输出里既有「错误」原因、也有「提示」给出的替代写法。
set -uo pipefail
cd "$(dirname "$0")/.."

cargo build --quiet
JRUST=./target/debug/jrust
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

pass=0
fail=0

check() {
    local name=$1 expect=$2 src=$3
    local f="$tmp/$name.jrust" out st
    printf '%s\n' "$src" >"$f"
    out=$("$JRUST" build "$f" 2>&1)
    st=$?
    if [ "$st" -eq 0 ]; then
        echo "FAIL $name：应当报错，却转译成功"
        fail=$((fail + 1))
        return
    fi
    if ! grep -qF -- "$expect" <<<"$out"; then
        echo "FAIL $name：输出缺少「$expect」（退出码 $st）"
        sed 's/^/    /' <<<"$out"
        fail=$((fail + 1))
        return
    fi
    if ! grep -qF -- "提示:" <<<"$out"; then
        echo "FAIL $name：报错里没有「提示:」"
        fail=$((fail + 1))
        return
    fi
    pass=$((pass + 1))
    echo "PASS $name"
}

# ---- Copy 标量：赋值本来就复制，没有所有权可搬 ----
check copy-int '属于 Copy 标量' 'public class CopyInt {
    public static void main(String[] args) {
        int a = 1;
        int b = move a;
    }
}'

check copy-double '属于 Copy 标量' 'public class CopyDouble {
    public static void main(String[] args) {
        double a = 1.5;
        double b = move a;
    }
}'

check copy-bool '属于 Copy 标量' 'public class CopyBool {
    public static void main(String[] args) {
        boolean f = true;
        boolean g = move f;
    }
}'

# ---- 没有可搬走所有权的右值 ----
check lit-string '只能作用于变量、字段或下标' 'public class LitStr {
    public static void main(String[] args) {
        String b = move "x";
    }
}'

check lit-int '只能作用于变量、字段或下标' 'public class LitInt {
    public static void main(String[] args) {
        int b = move 42;
    }
}'

check paren '只能作用于变量、字段或下标' 'public class ParenMove {
    public static void main(String[] args) {
        String a = "a";
        String b = move (a);
    }
}'

check new-expr '只能作用于变量、字段或下标' 'public class NewMove {
    public static void main(String[] args) {
        List<String> v = move new ArrayList<String>();
    }
}'

check call-result '只能作用于变量、字段或下标' 'public class CallMove {
    public static void main(String[] args) {
        List<String> v = new ArrayList<String>();
        String s = move v.get(0);
    }
}'

check nested '只能作用于变量、字段或下标' 'public class NestedMove {
    public static void main(String[] args) {
        String a = "a";
        String b = move move a;
    }
}'

# ---- 未声明变量 ----
check undeclared '未声明' 'public class Undeclared {
    public static void main(String[] args) {
        String b = move zzz;
    }
}'

# ---- 下标：rustc 不允许从 Vec 下标搬走所有权 ----
check index '下标搬走所有权' 'public class IndexMove {
    public static void main(String[] args) {
        List<String> v = new ArrayList<String>();
        String b = move v[0];
    }
}'

# ---- this.字段：self 是 &self，字段搬不出去 ----
check this-field '字段搬不出去' 'public class Holder {
    public String label;

    public String take() {
        return move this.label;
    }
}

public class ThisField {
    public static void main(String[] args) {
    }
}'

echo "----"
if [ "$fail" -ne 0 ]; then
    echo "失败 $fail 个，通过 $pass 个"
    exit 1
fi
echo "全部通过：$pass 个 move 非法边界都报了错并给出提示。"
