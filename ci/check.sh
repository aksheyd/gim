#!/usr/bin/env bash
set -euo pipefail

MAX_LINES="${MAX_LINES:-500}"
MAX_UNIT_TESTS="${MAX_UNIT_TESTS:-2}"
SRC_ROOT="${SRC_ROOT:-src}"

fail=0

rs_files() {
    git ls-files '*.rs'
}

src_files() {
    git ls-files "${SRC_ROOT}/*.rs" "${SRC_ROOT}/**/*.rs"
}

echo "check: rust files <= ${MAX_LINES} lines"
while IFS= read -r f; do
    [ -n "$f" ] || continue
    n=$(wc -l <"$f" | tr -d ' ')
    if [ "$n" -gt "$MAX_LINES" ]; then
        echo "  $f has $n lines (max $MAX_LINES)"
        fail=1
    fi
done < <(rs_files)

echo "check: no comments in rust sources"
if git ls-files '*.rs' | xargs grep -nE '(^|[[:space:]])//|/\*' >/tmp/gim-ci-comments.txt 2>/dev/null; then
    cat /tmp/gim-ci-comments.txt
    fail=1
fi
rm -f /tmp/gim-ci-comments.txt

echo "check: no unwrap/expect/panic in ${SRC_ROOT} production code"
while IFS= read -r f; do
    [ -n "$f" ] || continue
    if awk '/#\[cfg\(test\)\]/{exit} /unwrap\(|expect\(|panic!|todo!|unimplemented!/{print FILENAME":"NR":"$0; bad=1} END{exit bad+0}' "$f"; then
        :
    else
        fail=1
    fi
done < <(src_files)

echo "check: at most ${MAX_UNIT_TESTS} unit tests per ${SRC_ROOT} file"
while IFS= read -r f; do
    [ -n "$f" ] || continue
    n=$(grep -c '#\[test\]' "$f" || true)
    if [ "$n" -gt "$MAX_UNIT_TESTS" ]; then
        echo "  $f has $n unit tests (max $MAX_UNIT_TESTS); move extras to tests/"
        fail=1
    fi
done < <(src_files)

if [ "$fail" -ne 0 ]; then
    echo "check: failed"
    exit 1
fi
echo "check: ok"
