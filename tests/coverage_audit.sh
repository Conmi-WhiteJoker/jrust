#!/usr/bin/env bash
# 附录 A 覆盖率盘点（V2 阶段 A 附带项，见 cairn §14.9）。
#
# 目的：SPEC 附录 A 标 ✅ 只表示「映射规则写了」，不表示「有夹具真的跑到」。
# 标量加宽（JR-T9）就是这么藏了一整版——规则在表里，夹具里零覆盖。
# 本脚本对每个需求号给出一条**可判定的 grep 判据**，在夹具/示例语料里找证据。
#
# 判据设计原则：
#   - 用 `.jrust` 源码里的**写法**匹配，不用产物形态（产物形态会随重构变）。
#   - 每条判据尽量窄，宁可漏报也不要把无关写法算进来。
#   - 结果分 OK / MISS；MISS 不代表功能没实现，只代表「没有夹具覆盖」。
set -uo pipefail
cd "$(dirname "$0")/.."

# 语料：所有可能包含需求写法的 .jrust 源（不含负向夹具，它们本就该报错）。
corpus_files() {
    ls tests/conformance/L1/ok_*/main.jrust 2>/dev/null
    # 负向夹具也是覆盖证据：类型检查类需求（JR-T10/T12/T13/T14）只能靠负向夹具证明。
    ls tests/conformance/L1/neg_*/main.jrust 2>/dev/null
    ls examples/*/main.jrust 2>/dev/null
    find tests/multifile tests/l2/src tests/l2_cross -name '*.jrust' 2>/dev/null
}

CORPUS=$(mktemp)
trap 'rm -f "$CORPUS"' EXIT
# shellcheck disable=SC2046
cat $(corpus_files) > "$CORPUS" 2>/dev/null
LINES=$(wc -l < "$CORPUS")
FILES=$(corpus_files | wc -l)

ok=0
miss=0
missing_ids=""

check() {
    local id=$1 desc=$2 pat=$3
    if grep -qE -- "$pat" "$CORPUS"; then
        ok=$((ok + 1))
    else
        miss=$((miss + 1))
        missing_ids="$missing_ids $id"
        printf 'MISS  %-10s %s\n' "$id" "$desc"
    fi
}

echo "语料：$FILES 个文件 / $LINES 行（正向后缀 ok_* 夹具 + examples + 多文件工程）"
echo

# ---- 声明与类型 ----
check JR-D1   'public class → pub struct + impl'          '^\s*public class [A-Z]'
check JR-D1b  'private 成员'                                '^\s*private [A-Za-z]'
check JR-D4   'interface 声明'                              '^\s*public interface [A-Z]'
check JR-D4b  'implements 多接口'                           'implements [A-Z][A-Za-z]*(\s*,\s*[A-Z])'
check JR-D3   '构造器 this.f = p'                           'this\.[a-zA-Z]+ = [a-zA-Z]+;'
check JR-S6   '标量类型 int/long/double/float/boolean/char' '^\s*(public|private)?\s*(int|long|double|float|boolean|char)\s+[a-zA-Z]'
check JR-S6b  'Rust 原生标量 i32/i64/u8/usize/f32'          '\b(i32|i64|i128|isize|u8|u16|u32|u64|u128|usize|f32|f64)\s+[a-zA-Z]'
check JR-T9   '标量加宽 int → long/double'                  '(long|double)\s+[a-zA-Z]+\s*=\s*[a-zA-Z]+;'
check JR-L3   '数字下划线分隔'                              '[0-9]_[0-9]'
check JR-L6   'String 与字面量'                             'String\s+[a-zA-Z]+\s*='
check JR-T6   'List<T> / ArrayList'                         '(List<|ArrayList<)'
check JR-T8   'new ArrayList<>() / new HashMap<>()'         'new (ArrayList|HashMap|HashSet)<>\('
check JR-E15  'list.add / size / get / isEmpty'             '\.(add|size|get|isEmpty)\('
check JR-E4   'for (T x : v) 遍历'                          'for\s*\(\s*[A-Za-z]+\s+[a-zA-Z]+\s*:\s*'
check JR-E5   'C 风格 for'                                  'for\s*\(\s*int\s+[a-zA-Z]+\s*=\s*0\s*;'
check JR-T6b  'Map<K,V> / HashMap'                          '(Map<|HashMap<)'
check JR-E16  'map.put / containsKey / getOrNull'           '\.(put|containsKey|getOrNull)\('
check JR-E14  'println / print'                             '(println|print)\('
check JR-E13  '字符串 + 拼接'                               '"[^"]*"\s*\+|\+\s*"'
check JR-M1   'import java.*'                               'import java\.'
# 非 java.* 的 import（含外部 crate 与本地模块）。POSIX ERE 无负向断言，改用管道过滤。
if grep -qE '^import [a-z]' "$CORPUS" && grep -qE '^import (app|serde_json|std)\.' "$CORPUS"; then
    ok=$((ok + 1))
else
    miss=$((miss + 1)); missing_ids="$missing_ids JR-M2"
    printf 'MISS  %-10s %s\n' 'JR-M2' 'import 非 java.*'
fi
check JR-E1   'T x = e 局部声明'                            '^\s*(int|String|boolean|double|List<)'
check JR-E2   'i += 1 / i -= 1'                             '[a-zA-Z]+\s*(\+=|-=)\s*[0-9]'
check JR-E3   'if / else'                                   '^\s*if\s*\('
check JR-E6   'break / continue'                            '^\s*(break|continue);'
check JR-E9   'new 表达式'                                  'new [A-Z][A-Za-z]*\('
check JR-E10  '裸方法调用'                                  '^\s*[a-z][a-zA-Z]*\([^)]*\);\s*$'
check JR-E11  '接口单变量 new'                              '(Shape|Iface|Animal)\s+[a-zA-Z]+\s*=\s*new'
check JR-O1   '普通赋值（值语义 clone）'                    '^\s*String\s+[a-zA-Z]+\s*=\s*[a-zA-Z]+;'
check JR-O2   'move 表达式'                                 '\bmove\s+[a-zA-Z]'
check JR-T1   'T? 可空声明'                                 '[A-Za-z]+\?\s+[a-zA-Z]+\s*='
check JR-T1b  'x == null / != null'                         '[=!]=\s*null'
check JR-D7   'enum 带数据'                                 '^\s*[A-Z][a-zA-Z]*\([a-z]'
check JR-E7   'switch 语句'                                 '^\s*switch\s*\('
check JR-C1   'throws 声明'                                 '\bthrows\s+[A-Z]'
check JR-E17  '闭包 (a) -> ...'                             '\(?\s*[a-zA-Z]+\s*\)?\s*->'
check JR-E19  '? 传播'                                      '\w\([^)]*\)\?'
check JR-E12  'new T[n] 数组'                               'new (int|String|double)\['
check JR-T3   '泛型类 / 泛型方法'                           '(class|interface) [A-Z][A-Za-z]*<T|public <T>'
check JR-T4   '泛型上界 <T extends A>'                      '<T extends'
check JR-D8   'static final 常量'                           'static final'
check JR-D9   '@derive / @raw 注解'                         '@(derive|raw|inline|allow)\('
check JR-D10  '接口默认方法 / 静态方法'                     '^\s*(default|static)\s+[A-Za-z].*\('
check JR-C7   'panic / assert'                              '\b(panic|assert)\('
check JR-C9   'async 方法'                                  '\basync\s+[A-Za-z]'
check JR-C10  'await 表达式'                                '\bawait\b'
check JR-C11  'spawn 块'                                    '\bspawn\s*\{'
check JR-C12  'synchronized'                                '\bsynchronized\b'
check JR-C13  '@shared 字段'                                '@shared\s+[A-Za-z]'
check JR-E18  '迭代器链 stream/map/filter'                  '\.(stream|collectList)\('
check JR-M3   'package 声明'                                '^package [a-z]'
check JR-X1   'raw 内联'                                    '\braw\s*\{'
check JR-T2   '借用形参 @ref / @mut'                        '@(ref|mut)\s+[A-Z]'

# ---- 类型检查（v1.41–v1.44）----
# 这些检查的「证据」是负向夹具——它们的存在即证明规则被跑到。
check JR-T10   '未知类型名报 JR6001'                        '^\s*var\s+[a-zA-Z]+\s*='
check JR-T12   '未知方法报 JR6004'                          '\.substring\('
check JR-T13   '非 bool 条件报 JR6009'                      'if\s*\(\s*[a-z][a-zA-Z]*\s*\)'
check JR-T14   '赋值/返回类型不匹配报 JR6007/8'              'int\s+[a-z]+\s*=\s*"'
check JR-T16   '二元运算标量提升'                            '[a-z]\s*[+*]\s*[0-9]+\.[0-9]'

echo
echo "覆盖：$ok 项有夹具 / $miss 项无夹具"
if [ -n "$missing_ids" ]; then
    echo "无夹具的需求号：$missing_ids"
fi
[ "$miss" -eq 0 ]
