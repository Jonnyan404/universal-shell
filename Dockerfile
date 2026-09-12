# syntax=docker/dockerfile:1
# us-web — 从 Universal Shell GitHub Release 下载并运行 headless CLI 服务端
#（不在镜像内编译，直接从 release 拉取预编译的 universal-shell-cli 安装包。
#  该包在 ubuntu-24.04 runner 上以标准 glibc 动态链接构建，要求 glibc>=2.39，
#  故运行时用 debian:trixie（glibc 2.41）的标准精简镜像，小且兼容）
#
# 构建（单架构：ARCH 取 x86_64 或 arm64，对应 release asset 的 -linux-<ARCH> 后缀）：
#   docker build -t universal-shell:amd64 . --build-arg ARCH=x86_64
#   docker build -t universal-shell:arm64 . --build-arg ARCH=arm64   # 在 arm 主机上
#
# 构建（多架构：交给 buildx，自动按目标平台推断 ARCH，无需手动指定）：
#   docker buildx build --platform linux/amd64,linux/arm64 -t universal-shell:0.1.8 .
#
# 运行（手工指定版本，如 0.1.8）：
#   docker run --rm -p 45990:45990 \
#     -v us-data:/data \
#     -e US_WEB_BIND=0.0.0.0 \
#     -e US_WEB_PORT=45990 \
#     -e US_WEB_TOKEN=MyPassword123 \
#     universal-shell:amd64
#
# 可用环境变量：
#   US_WEB_BIND  监听 IP，默认 0.0.0.0（否则 loopback 只在容器内可达）
#   US_WEB_PORT  监听端口，默认 45990
#   US_WEB_TOKEN 访问密码；不设置则首次启动自动随机生成并写入配置
# 数据目录默认 /data（容器内），通过 -v 挂载卷持久化。
# 注意：容器无桌面会话，「远程文件选择」dialog 依赖桌面 portal，容器内不可用（属预期）。

ARG VERSION=0.2.2

# ---- 下载阶段：从 Release 拉取预编译二进制 ----
FROM debian:trixie-slim AS fetch
ARG VERSION
# buildx 提供的目标平台（arm64/amd64）；手动构建未指定 ARCH 时据此推断
ARG TARGETARCH
# 手动指定 release asset 后缀；留空则按 TARGETARCH 自动推断（amd64→x86_64）
ARG ARCH=
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && if [ -n "$ARCH" ]; then REL_ARCH="$ARCH"; else \
         case "$TARGETARCH" in \
           arm64) REL_ARCH="arm64" ;; \
           *)     REL_ARCH="x86_64" ;; \
         esac; \
       fi \
    && curl -fsSL -o /us-web \
       "https://github.com/Jonnyan404/universal-shell/releases/download/v${VERSION}/universal-shell-cli-${VERSION}-linux-${REL_ARCH}" \
    && chmod +x /us-web

# ---- 运行阶段 ----
FROM debian:trixie-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
       ca-certificates libwayland-client0 libxkbcommon0 \
    && rm -rf /var/lib/apt/lists/* \
    && useradd -m -d /data usweb
WORKDIR /data
COPY --from=fetch /us-web /usr/local/bin/us-web

# 入口脚本：把 US_WEB_BIND / US_WEB_PORT / US_WEB_TOKEN 环境变量映射成 us-web 的命令行参数（带默认值）
# 必须在 USER 切换之前（否则非 root 无写入 /usr/local/bin 权限）
# 用 set -- 逐项组参，密码含空格/特殊字符也不会被 shell 拆开
RUN printf '%s\n' \
      '#!/bin/sh' \
      'set -e' \
      'BIND="${US_WEB_BIND:-0.0.0.0}"' \
      'PORT="${US_WEB_PORT:-45990}"' \
      'set -- us-web --bind "$BIND" --port "$PORT"' \
      'if [ -n "${US_WEB_TOKEN:-}" ]; then' \
      '  set -- "$@" --token "$US_WEB_TOKEN"' \
      'fi' \
      'exec "$@"' \
    > /usr/local/bin/docker-entrypoint.sh && chmod +x /usr/local/bin/docker-entrypoint.sh

# 到此为止都是 root；之后切到普通用户
USER usweb

ENV HOME=/data XDG_DATA_HOME=/data/.local/share
EXPOSE 45990
VOLUME /data
ENTRYPOINT ["/usr/local/bin/docker-entrypoint.sh"]
