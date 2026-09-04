#!/usr/bin/env bash
# 打包 universal-shell 两版可分发产物。
#
# egui / tauri 均按平台产出自带安装包（单一构建主机只能原生产出自己系统的包，
# 与 tauri bundles "all" 行为一致；Linux .deb/.rpm/.AppImage 需 Linux 环境）：
#   macOS:  egui=.app+.dmg(原生单架构)          tauri=.app+.dmg(.app 目录)
#   Windows: egui=.exe+.msi(xwin+WiX)          tauri=.msi+.exe
#   Linux:  egui 单二进制(暂)                  tauri=.deb/.rpm/.AppImage
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

APP="universal-shell-egui"
ICONS="assets/icons/egui-bundle"

echo "==> version: $VERSION, os: $OS, arch: $ARCH"

rm -rf dist
mkdir -p dist

have() { command -v "$1" >/dev/null 2>&1; }

echo "==> build egui installers"
EGUI="dist/egui/$OS"
if [ "$OS" = "darwin" ]; then
  cargo build --release -p app-egui

  mkdir -p "$EGUI"
  APPNAME="$EGUI/$APP.app"
  mkdir -p "$APPNAME/Contents/MacOS" "$APPNAME/Contents/Resources"
  cp target/release/$APP "$APPNAME/Contents/MacOS/$APP"
  cp "$ICONS/icon.icns" "$APPNAME/Contents/Resources/icon.icns"
  cat > "$APPNAME/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Universal Shell</string>
  <key>CFBundleDisplayName</key><string>Universal Shell</string>
  <key>CFBundleIdentifier</key><string>com.universal.shell.egui</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleExecutable</key><string>$APP</string>
  <key>CFBundleIconFile</key><string>icon</string>
  <key>LSMinimumSystemVersion</key><string>12.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>LSApplicationCategoryType</key><string>public.app-category.developer-tools</string>
</dict>
</plist>
PLIST
  chmod +x "$APPNAME/Contents/MacOS/$APP"

  STAGE="$EGUI/.dmg-stage"
  mkdir -p "$STAGE"
  cp -R "$APPNAME" "$STAGE/"
  ln -s /Applications "$STAGE/Applications"
  hdiutil create -volname "Universal Shell $VERSION" -srcfolder "$STAGE" \
    -ov -format UDZO "$EGUI/$APP-$VERSION-$ARCH.dmg" >/dev/null
  rm -rf "$STAGE"

elif [ "$OS" = "mingw" ] || [ "$OS" = "windows" ] || [ "$OSTYPE" = "msys" ] || [ "$OSTYPE" = "cygwin" ]; then
  # 按宿主架构选 MSVC target（原生 Windows ARM64 runner 也能直接编译 aarch64）；
  # 可用 EGUI_TARGET 显式覆盖（例如在 macOS 上用 xwin 交叉出其它 arch）。
  # 注意：Git Bash 在 ARM64 Windows 上跑在 x86_64 仿真下，uname -m 返回 x86_64，
  # 但 uname -s 会带 -arm64 后缀（如 mingw64_nt-10.0-26200-arm64），
  # 以此为可靠判据；PROCESSOR_ARCHITECTURE 在 MSYS 下也是 AMD64，不可靠。
  if [ -n "${EGUI_TARGET:-}" ]; then
    TGT="$EGUI_TARGET"
  else
    case "$OS" in
      *-arm64) TGT=aarch64-pc-windows-msvc ;;
      *)       TGT=x86_64-pc-windows-msvc ;;
    esac
  fi
  echo "==> [Windows] target: $TGT"
  # macOS 交叉 .exe 用 cargo-xwin；原生 Windows 直接 cargo build
  if [ "$(uname -s)" = "Darwin" ]; then
    cargo xwin build --release --target "$TGT" -p app-egui
    BIN="target/$TGT/release/$APP.exe"
  else
    cargo build --release --target "$TGT" -p app-egui
    BIN="target/$TGT/release/$APP.exe"
  fi
  mkdir -p "$EGUI"
  cp "$BIN" "$EGUI/$APP-$VERSION-$TGT.exe"

  # .msi 用 WiX (candle/light) + cargo-wix：WiX 本身仅 Windows 原生，
  # 因此在 macOS 上交叉出 .msi 不可行，仅当 WiX 在 PATH 时才产出
  if { have candle || have wix; } && [ -f wix/main.wxs ]; then
    mkdir -p "$EGUI"
    (cd app-egui && cargo wix --nocapture --no-build -p app-egui \
      --target "$TGT" --target-bin-dir "../../../target/$TGT/release" \
      -o "../$EGUI/$APP-$VERSION-$TGT.msi")
  else
    echo "!! WiX Toolset(candle) 不在 PATH：egui 仅产出 .exe（.msi 需在 Windows/WiX 环境构建）"
  fi
else
  # Linux：egui 暂为单二进制，安装包留 CI（与 tauri 原生 .deb/.rpm/.AppImage 一致）
  cargo build --release -p app-egui
  mkdir -p "$EGUI"
  cp target/release/$APP "$EGUI/$APP-$VERSION-$OS-$ARCH"
  chmod +x "$EGUI/$APP-$VERSION-$OS-$ARCH"
fi

echo "==> build tauri bundle"
# tauri 仅在 Linux/macOS 打包（Windows 上无 cargo-tauri/NSIS 支持，跳过）；
# 即便失败也保留已千辛万苦产出的 egui 安装包，仅记警告
case "$OS" in
  darwin|linux)
    # 优先用已装的 cargo-tauri-cli；否则回退到 app-tauri 的 node_modules 里的 @tauri-apps/cli
    if (cd app-tauri && cargo tauri build) || (cd app-tauri && npx tauri build); then
      if [ -d target/release/bundle ]; then
        cp -R target/release/bundle dist/bundle
      fi
    else
      echo "!! tauri bundle 打包失败（egui 产物已生成，继续收尾）"
    fi
    ;;
  *)
    echo "!! 当前平台($OS)无 tauri 打包环境，跳过 tauri bundle"
    ;;
esac

echo "==> sha256"
: > dist/sha256.txt
if have sha256sum; then
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
