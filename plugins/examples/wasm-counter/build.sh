#!/usr/bin/env sh
# 构建 wasm-counter 示例插件
# 需要 wabt（wat2wasm）或 wasm-tools，二选一
set -e
cd "$(dirname "$0")"

if command -v wat2wasm >/dev/null 2>&1; then
	wat2wasm counter.wat -o counter.wasm
elif command -v wasm-tools >/dev/null 2>&1; then
	wasm-tools parse counter.wat -o counter.wasm
else
	echo "需要 wat2wasm (wabt) 或 wasm-tools" >&2
	exit 1
fi
echo "已生成 counter.wasm"
