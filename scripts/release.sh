#!/usr/bin/env bash
# 一键发布：统一提升三处版本 -> commit -> 打 vX.Y.Z tag -> push (分支+tag)。
#
# 版本唯一来源 = 本脚本参数。它会把以下文件全部写到同一个版本号，消灭漂移：
#   Cargo.toml                    根 workspace.package.version（App 内"当前版本"/更新检查）
#   app-tauri/src-tauri/tauri.conf.json   tauri 安装包版本（本地打包；CI 仍以 release tag 为准覆盖）
#   app-tauri/package.json          npm 元数据版本
#
# 用法:
#   ./scripts/release.sh [v]X.Y.Z        # 例: ./scripts/release.sh 0.1.5
#
# 前置: 工作树必须干净（有未提交改动会拒绝，请先 commit/stash）。
# 发布后 CI 需在 GitHub 上创建 Release 才触发（tag 本身不触发）：
#   gh release create v0.1.5 --generate-notes
set -euo pipefail

cd "$(dirname "$0")/.."

# ---- 参数校验 ----
if [ "$#" -ne 1 ]; then
  echo "用法: $0 [v]X.Y.Z"; exit 2
fi
VER="${1#v}"
if ! [[ "$VER" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "错误: 版本号需形如 X.Y.Z（可带 v 前缀），实际: $1"; exit 2
fi
TAG="v$VER"

# ---- 干净工作树校验 ----
if [ -n "$(git status --porcelain)" ]; then
  echo "错误: 工作树不干净，请先 commit 或 stash："
  git status --short
  exit 1
fi

BRANCH="$(git branch --show-current)"

# ---- 统一改三处版本（BSD/GNU sed 均兼容，先备份再删备份，改后逐个核验） ----
bump() { # bump <file> <sed-expr> <grep-expect> <description>
  local f="$1" expr="$2" expect="$3" desc="$4"
  sed -i.bak "$expr" "$f" && rm -f "$f.bak"
  grep -q "$expect" "$f" || { echo "错误: $desc 未改成 $VER（$f）"; exit 1; }
  echo "  ✓ $desc -> $VER  ($f)"
}

bump Cargo.toml \
  "s/^version = \"[^\"]*\"/version = \"$VER\"/" \
  "^version = \"$VER\"$" \
  "workspace.package.version"

bump app-tauri/src-tauri/tauri.conf.json \
  "s/\"version\": \"[^\"]*\"/\"version\": \"$VER\"/" \
  "\"version\": \"$VER\"" \
  "tauri.conf.json version"

bump app-tauri/package.json \
  "s/\"version\": \"[^\"]*\"/\"version\": \"$VER\"/" \
  "\"version\": \"$VER\"" \
  "package.json version"

echo "==> git diff 确认"
git diff --stat Cargo.toml app-tauri/package.json app-tauri/src-tauri/tauri.conf.json

# ---- commit + tag + push ----
git add Cargo.toml app-tauri/package.json app-tauri/src-tauri/tauri.conf.json
git commit -m "Bump version to $VER"
git tag -a "$TAG" -m "Release $TAG"

echo "==> push 分支 ($BRANCH) 与 tag ($TAG)"
git push origin "$BRANCH" "$TAG"

echo "==> 完成: $TAG"
echo "下一步（二选一，均会触发 CI 构建并发布资产）:"
echo "  gh release create $TAG --generate-notes"
echo "  或网页端 New release -> Choose tag: $TAG"