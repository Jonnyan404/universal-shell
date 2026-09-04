#!/usr/bin/env bash
# 打包 universal-shell 两版可分发产物。
#
# egui / tauri 均按平台产出自带安装包（单一构建主机只能原生产出自己系统的包，
# 与 tauri bundles "all" 行为一致；Linux .deb/.rpm/.AppImage 需 Linux 环境）：
#   macOS:  egui=.app+.dmg(arm64+x86_64 通用)  tauri=.app+.dmg(.app 目录)
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
  cargo build --release --target aarch64-apple-darwin -p app-egui
  cargo build --release --target x86_64-apple-darwin -p app-egui

  mkdir -p "$EGUI"
  APPNAME="$EGUI/$APP.app"
  mkdir -p "$APPNAME/Contents/MacOS" "$APPNAME/Contents/Resources"
  lipo -create -output "$APPNAME/Contents/MacOS/$APP" \
    target/aarch64-apple-darwin/release/$APP \
    target/x86_64-apple-darwin/release/$APP
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
    -ov -format UDZO "$EGUI/$APP-$VERSION.dmg" >/dev/null
  rm -rf "$STAGE"

elif [ "$OS" = "mingw" ] || [ "$OS" = "windows" ] || [ "$OSTYPE" = "msys" ] || [ "$OSTYPE" = "cygwin" ]; then
  TGT=x86_64-pc-windows-msvc
  # macOS 交叉 .exe 用 cargo-xwin；原生 Windows 直接 cargo build
  if [ "$(uname -s)" = "Darwin" ]; then
    cargo xwin build --release --target "$TGT" -p app-egui
    BIN="target/$TGT/release/$APP.exe"
  else
    cargo build --release --target "$TGT" -p app-egui
    BIN="target/$TGT/release/$APP.exe"
  fi
  mkdir -p "$EGUI"
  cp "$BIN" "$EGUI/$APP-$VERSION.exe"

  # .msi 用 WiX (candle/light) + cargo-wix：WiX 本身仅 Windows 原生，
  # 因此在 macOS 上交叉出 .msi 不可行，仅当 WiX 在 PATH 时才产出
  if have candle && [ -f wix/main.wxs ]; then
    (cd app-egui && cargo wix --nocapture --no-build \
      --target "$TGT" --target-bin-dir "../../../target/$TGT/release" \
      -o "../$EGUI/$APP-$VERSION.msi")
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
find dist -maxdepth 4 -type f | sed 's#^#  #' | sort
