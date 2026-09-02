#!/usr/bin/env bash
# D2: 打包 universal-shell 两版可分发产物。
#
# 产物（按平台不同）：
#   egui:  dist/universal-shell-egui-<version>-<os>-<arch>[.exe]
#   tauri: dist/bundle/ 下 由系统原生工具生成的安装包(.dmg/.msi/.AppImage 等) 及 .app 目录
# 同时输出每个产物的 SHA256（写入 dist/sha256.txt），便于发布与校验。
#
# 用法:
#   ./build-release.sh [version]
set -euo pipefail
cd "$(dirname "$0")/.."

VERSION="${1:-$(grep '^version' Cargo.toml | head -1 | sed -E 's/.*= *"([^"]+)".*/\1/')}"
OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
ARCH="$(uname -m)"
export MACOSX_DEPLOYMENT_TARGET=12.0

echo "==> version: $VERSION, os: $OS, arch: $ARCH"

rm -rf dist
mkdir -p dist

echo "==> build egui single binary"
cargo build --release -p app-egui
EGUI="dist/universal-shell-egui-$VERSION-$OS-$ARCH"
cp target/release/universal-shell-egui "$EGUI"
if [ "$OS" = "darwin" ]; then
  # egui 无打包需求，单二进制即可；补 x 权限
  chmod +x "$EGUI"
fi

echo "==> build tauri bundle"
(cd app-tauri && cargo tauri build)
if [ -d target/release/bundle ]; then
  cp -R target/release/bundle dist/bundle
fi

echo "==> sha256"
: > dist/sha256.txt
find dist -type f -not -name sha256.txt -not -name '.DS_Store' -print0 | while IFS= read -r -d '' f; do
  shasum -a 256 "$f" | sed "s#dist/##" >> dist/sha256.txt
done
cat dist/sha256.txt
echo "==> done: $(pwd)/dist"
find dist -maxdepth 3 -type f | sed 's#^#  #' | sort