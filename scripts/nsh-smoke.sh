#!/usr/bin/env sh
set -eu

arch="${1:-x86_64}"
log="${NORX_SHELL_SMOKE_LOG:-build/nsh-${arch}-smoke.log}"

case "$arch" in
    x86_64|aarch64) ;;
    *)
        echo "usage: $0 [x86_64|aarch64]" >&2
        exit 2
        ;;
esac

mkdir -p "$(dirname "$log")"
rm -f "$log"

set +e
NORX_SHELL_SMOKE_LOG="$log" python3 ./scripts/nsh-smoke.py "$arch" >"$log" 2>&1
status=$?
set -e

if [ "$status" -ne 124 ]; then
    echo "nsh smoke expected timeout after clean shell exit, got exit=$status" >&2
    cat "$log" >&2
    exit 1
fi

grep -F 'nsh: parse: unclosed quote' "$log" >/dev/null
grep -F 'nsh: command not found: missing-command' "$log" >/dev/null
grep -F 'quickinit: PID 1 hand-off complete; child reaped' "$log" >/dev/null
grep -F 'nsh: external userspace ELF exited cleanly' "$log" >/dev/null

echo "nsh $arch smoke passed; log=$log"
