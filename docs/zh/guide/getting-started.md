# 入门指南

本指南带你完成 `minecraft-mcp-rs` —— 一个通过模型上下文协议（MCP）控制的
Minecraft 机器人 —— 的构建、运行与测试。

## 前置条件

- [Rust nightly](https://rustup.rs/) —— 在 `rust-toolchain.toml` 中固定
  （edition 2024；azalea 0.15.1 的构建脚本要求 nightly）
- 一个 **Minecraft Java Edition 1.21.11** 服务器（本地或远程均可）——**唯一受支持的版本**
- 一个 **MCP 客户端** —— Claude Desktop、Cursor 或任何兼容 MCP 的 LLM 宿主

> 本工具**仅支持 Minecraft Java Edition 1.21.11**，其他服务器版本均无法工作。如果你的服务器运行不同版本，请安装匹配的 `minecraft-mcp-rs` 发行版（参见 [npm 安装](../npm#版本兼容性)）。

| Minecraft 服务器版本 | minecraft-mcp-rs 版本 |
|----------------------|-----------------------|
| 1.21.11              | 1.1.4                 |

## 构建

```bash
cargo build
```

## 运行

不带参数运行 `cargo run` 会打印帮助并退出（退出码 0）。启动桌面 UI 需要显式传入 `--gui`：

```bash
cargo run -- --gui
```

无头运行（不打开窗口，MCP 传输关闭时退出进程）使用 `--headless`；`--stdio` 单独使用时也隐含无头模式。在设置面板中选择 MCP 传输方式：

- **stdio** —— MCP 服务器监听 stdin/stdout（Claude Desktop / Cursor 的默认方式）。
- **HTTP** —— MCP 服务器仅绑定到 `127.0.0.1`；设置端口，并按需启用 Bearer 令牌鉴权（默认关闭；启用时默认令牌为随机 UUID v4，可在设置面板中覆盖）。MCP 配置面板会生成匹配的 JSON 配置，方便复制到你的 MCP 客户端。

默认情况下，机器人会尝试以 `AI_Bot` 的身份连接 `127.0.0.1:25565`。可以在
UI 面板中或启动前通过环境变量调整设置（参见[配置](../config)）。

## 测试

```bash
cargo test                         # 所有测试
cargo test --lib                   # 仅单元测试
cargo test --test integration      # 基于 mock 的集成测试
cargo test --test proptest         # 属性测试
```

单元测试位于每个源文件底部的 `#[cfg(test)] mod tests` 中。`tests/integration.rs`
中的集成测试使用 mock（无需真实 MC 服务器），`tests/proptest.rs` 中的属性测试
使用 `proptest` crate。
