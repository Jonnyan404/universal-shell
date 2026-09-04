# 三方源示例（example-third-party）

这是一个**自建模板源的示例**，展示如何托管自己的模板仓库，供 universal-shell 拉取。

> 官方源就是按同样的两级结构组织的（本目录的 `registry/`）。照抄结构、换成你自己的模板即可。

## 目录结构（两级惰性拉取）

```
registry/example-third-party/
├── manifests.json            # ① 索引：分类 + 每个模板的元信息
└── templates/
    └── fd.json               # ② 具体模板：id 为 fd 的 Program 定义
```

- 壳只拉 `manifests.json` 索引（轻量），用户点某个模板时才惰性拉取 `templates/<id>.json`。
- `manifests.json` 里 `templates[].id` 必须与 `templates/<id>.json` 的 `id` 一致。
- **`revision` 每次更新内容后要改**（建议 `rev-<unix时间戳>`），否则本地缓存可能不刷新。

## 如何自建

1. 把本目录结构上传到任意静态托管（如 GitHub Pages、对象存储、`python -m http.server`）。
2. 保守起见，模板内 `bind` 等监听字段默认 `127.0.0.1`，端口用高位端口。
3. 在 universal-shell 的「模板库 → 源」设置里，把 `https://你的域名/路径/` 加为源。
   - 可选：配置源对应的 Ed25519 公钥；命中公钥的源会强制验签 `manifests.sig`。

## 模板字段说明（Program）

`templates/<id>.json` 各字段：

| 字段 | 说明 |
| --- | --- |
| `id` / `name` / `description` | 标识与展示文本 |
| `repo` | GitHub `owner/repo`，用于解析最新版本与下载链接 |
| `binary` | 解压后可执行文件内部名 |
| `assets.<os>.candidates` | 按顺序尝试的下载文件名模板（支持 `{name}` `{version}` `{arch}`）|
| `assets.<os>.mode` | `single`（单裸二进制）/ `whole`（整包解压）|
| `assets.<os>.member` | 压缩包内可执行文件成员名 |
| `arch_map` / `os_map` | 把壳的架构/系统名映射成 release 里的命名 |
| `fields` | 用户填写字段（`string` / `file` / `directory` / `boolean` / `autostart`）|
| `args` | 启动参数模板，`{字段key}` 会被替换为用户填写的值 |

> **自启动**：自启动已由壳统一管理（`program-autostart.json`），模板**不必**再声明 `autostart` 字段；
> 壳会对每个程序（含内置/用户/该源导入的）独立提供开机自启开关。

## 示例模板说明

`templates/fd.json` 基于真实项目 `sharkdp/fd`（Rust 写的 find 替代品），其 GitHub
release 命名形如 `fd-v10.2.0-aarch64-apple-darwin.tar.gz`，与 `candidates` 一一对应。
导入后不联网也能在壳里按字段填 `{pattern}`、`{dir}` 启动。

## 参考

- 生成官方模板：`cargo run -p shared --example generate_registry`
- 校验模板合法性：`cargo run -p shared --example verify_templates`
- 本示例文件结构若与官方不一致，以 `shared/src/registry.rs` / `shared/src/config.rs` 为准。
