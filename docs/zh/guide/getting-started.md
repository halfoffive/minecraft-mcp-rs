# 入门指南

本指南带你完成 `minecraft-mcp-rs` —— 一个通过模型上下文协议（MCP）控制的
Minecraft 机器人 —— 的构建、运行与测试。

## 前置条件

- [Rust nightly](https://rustup.rs/) —— 在 `rust-toolchain.toml` 中固定
  （edition 2024；azalea 0.15.1 的构建脚本要求 nightly）
- 一个 **Minecraft Java Edition 1.21.11** 服务器（本地或远程均可）
- 一个 **MCP 客户端** —— Claude Desktop、Cursor 或任何兼容 MCP 的 LLM 宿主

## 构建

```bash
cargo build
```

## 运行

```bash
cargo run
```

这会同时启动 MCP 服务器和 egui 桌面 UI。在设置面板中选择 MCP 传输方式：

- **stdio** —— MCP 服务器监听 stdin/stdout（Claude Desktop / Cursor 的默认方式）。
- **HTTP** —— MCP 服务器仅绑定到 `127.0.0.1`；设置端口和 Bearer 令牌
  （默认为项目名 `minecraft-mcp-rs`）。MCP 配置面板会生成匹配的 JSON 配置，
  方便复制到你的 MCP 客户端。

默认情况下，机器人会尝试以 `AI_Bot` 的身份连接 `127.0.0.1:25565`。可以在
UI 面板中或启动前通过环境变量调整设置（参见[配置](./configuration)）。

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
