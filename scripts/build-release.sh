#!/usr/bin/env bash
# 打包 universal-shell 可分发产物（Tauri 安装包）。
#
# 单一构建主机只能原生产出自己系统的包，与 tauri bundles "all" 行为一致
# （Linux .deb/.rpm/.AppImage 需 Linux 环境）：
#   macOS:   .app + .dmg
#   Windows: .msi + .exe（nsis）
#   Linux:   .deb / .rpm / .AppImage
# 同时输出每个产物的 SHA256（写入 dist/sha256.txt），便于发布与校验。
#
# 注：egui 备用界面已归档（见 ROADMAP「决策记录」），本脚本不再产出 egui 产物。
#
# 用法:
#   ./build-release.sh [version]
set -euo pipefail
cd "$(dirname "$0")/.."

VERSION="${1:-$(grep '^version' Cargo.toml | head -1 | sed -E 's/.*= *"([^"]+)".*/\1/')}"
OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
ARCH="$(uname -m)"
export MACOSX_DEPLOYMENT_TARGET=12.0
# 注入构建期版本：App 内版本/自更新检查与本次打包版本一致（shared::version::build_version）
export UNIVERSAL_SHELL_VERSION="$VERSION"

echo "==> version: $VERSION, os: $OS, arch: $ARCH"

rm -rf dist
mkdir -p dist

echo "==> build tauri bundle"
# tauri 三端都支持：Linux/macOS 原生出，Windows 用 nsis+msi（NSIS 自动下载、WiX 需已装）
case "$OS" in
  darwin|linux)
    # 优先用已装的 cargo-tauri-cli；否则回退到 app-tauri 的 node_modules 里的 @tauri-apps/cli
    # 均用 --config 覆盖 conf 里的写死版本，保证安装包版本与本次 VERSION 一致（与 CI 行为对齐）
    if (cd app-tauri && cargo tauri build --config "{\"version\":\"$VERSION\"}") || (cd app-tauri && npx tauri build --config "{\"version\":\"$VERSION\"}") ; then
      if [ -d target/release/bundle ]; then
        cp -R target/release/bundle dist/bundle
      fi
    else
      echo "!! tauri bundle 打包失败"
      exit 1
    fi
    ;;
  mingw*|msys*|cygwin*)
    # Windows 原生：nsis+msi（msi 依赖 WiX）
    if (cd app-tauri && cargo tauri build --bundles nsis,msi --config "{\"version\":\"$VERSION\"}") || (cd app-tauri && npx tauri build --bundles nsis,msi --config "{\"version\":\"$VERSION\"}") ; then
      if [ -d target/release/bundle ]; then
        cp -R target/release/bundle dist/bundle
      fi
    else
      echo "!! tauri bundle 打包失败"
      exit 1
    fi
    ;;
  *)
    echo "!! 当前平台($OS)无 tauri 打包环境，退出"
    exit 1
    ;;
esac

echo "==> sha256"
: > dist/sha256.txt
if command -v sha256sum >/dev/null 2>&1; then
  HASH="sha256sum"
else
  HASH="shasum -a 256"
fi
find dist -type f -not -name sha256.txt -not -name '.DS_Store' -print0 | while IFS= read -r -d '' f; do
  $HASH "$f" | sed "s#dist/##" >> dist/sha256.txt
done
cat dist/sha256.txt
echo "==> done: $(pwd)/dist"
find dist -maxdepth 4 -type f | sed 's#^#  #' | sort
