#!/usr/bin/env bash
set -euo pipefail

PROJECT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
cd "$PROJECT_DIR"

for tool in bun cargo rustc; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    printf '缺少必需工具：%s，请先安装后重试。\n' "$tool" >&2
    exit 1
  fi
done

bun install --frozen-lockfile
exec bun run desktop:dev "$@"
