# 从源码构建

本指南介绍如何从源码构建、运行和测试 `minecraft-mcp-rs`。如果你只是想
**使用** 这个服务器，可以完全跳过本页 —— [npm 安装](../npm) 页面用
`npx`/`bunx` 即可运行，无需任何 Rust 工具链。

## 前置条件

- [Rust nightly](https://rustup.rs/) —— 具体版本由 `rust-toolchain.toml`
  按日期固定；较新的 `rustup` 在进入仓库目录时会自动切换。
- Node.js ≥ 18 —— 仅文档站（`npm run docs:dev`）和 npm 启动器需要；
  编译二进制本身不需要。
- 一台 **Minecraft Java Edition 1.21.11** 服务器（本地或远程）——
  **唯一支持的版本**。azalea 0.15.1 精确对应这个协议版本；其他版本的
  服务器会在登录阶段拒绝该 bot。

## 为什么固定 nightly

本 crate 基于 Rust edition 2024 构建，且 azalea 0.15.1 的构建脚本强制要求
nightly 编译器。工具链文件固定的是 nightly 的**日期**
（如 `nightly-2026-05-28`）而不是漂移的 `nightly` 通道：裸 `nightly`
曾让上游新编译器在 CI 中破坏 `azalea-core`（`FixedBitSet` 处的 E0284）。
升级触及 azalea 的依赖时，应在同一次变更中顺带提升固定日期，并确认下方
全套质量门仍然通过。

## 构建

```bash
cargo build          # dev profile
cargo build --release
```

dev profile 对工作区代码使用 `opt-level = 1`，但依赖保持
`opt-level = 3`：迭代更快，同时 azalea 的热路径（区块解码、寻路）
运行时依然可用。

## 运行

```bash
cargo run                 # 打印用法后退出
cargo run -- --gui        # 桌面 UI（egui 窗口）
cargo run -- --headless --stdio   # 无头模式，MCP 走 stdio
cargo run -- --headless           # 无头模式，MCP 走 HTTP（默认）
```

MCP 客户端的接入见[快速开始](../guide/getting-started)，全部
`MINECRAFT_MCP_*` 环境变量见[配置](../config)。

## 测试

```bash
cargo test                         # 以下全部
cargo test --lib                   # 仅单元测试
cargo test --test integration      # mock 集成测试
cargo test --test proptest         # 属性测试
```

单元测试位于每个源码文件底部的 `#[cfg(test)] mod tests`。集成测试全程
使用 mock（不连接真实 Minecraft 服务器），整套离线一分钟内跑完。

## Lint 与文档门禁

CI 会执行以下所有检查；提交 PR 前请先在本地跑一遍：

```bash
cargo fmt                       # 格式化
cargo clippy --all-targets      # 必须零警告
cargo doc --no-deps             # 必须零警告（[lints.rustdoc] deny 基线）
```

rustdoc 门禁拒绝失效的 intra-doc 链接、私有项误链接和冗余显式目标 ——
私有项请写成纯代码片段（`` `item` ``），不要用 `[]` 链接。

## 交叉编译 / 发布产物

发布二进制由 CI（`.github/workflows/release.yml`）构建，覆盖 Linux
x64/arm64、macOS arm64、Windows x64/arm64，并装入 npm 平台包。本地复现：

```bash
cargo build --release
# target/release/minecraft-mcp-rs(.exe)
```

ARM 目标由原生 ARM runner 构建，无需交叉工具链。

## 常见构建问题

| 现象 | 原因 / 处理 |
|------|-------------|
| `azalea-core` 内报 `E0284`（`FixedBitSet`） | 比 pin 更新的 nightly 破坏了上游。使用固定日期的 nightly（仓库内 `rust-toolchain.toml` 会自动生效）；提升 pin 属于依赖升级任务，需单独验证。 |
| `azalea requires a nightly compiler` | 当前激活的是 stable/默认工具链。用 `rustc -vV` 确认仓库内报告的是被固定的 nightly 日期。 |
| debug 构建运行很慢 | 多半是依赖的 opt-level 被调低；保留 `[profile.dev.package."*"] opt-level = 3`。 |
| `link.exe not found`（Windows） | 安装 Visual Studio Build Tools 并勾选 C++ 工作负载。 |
