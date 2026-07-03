# 架构

`minecraft-mcp-rs` 采用分层结构 **types → logic → state → bot → mcp → ui**，
全部位于同一个 crate 中。

```
┌──────────────────────────────────────────────────┐
│                   egui Desktop UI               │
│  ┌──────────┐  ┌──────────┐                     │
│  │  Status  │  │ Settings │                     │
│  └────┬─────┘  └────┬─────┘                     │
│       └──────┬───────┘                          │
│              │ reads/writes                      │
│              ▼                                   │
│       ┌──────────┐                               │
│       │SharedState│  (ArcSwap + RwLock + Atomics)│
│       └────┬─────┘                               │
└────────────┼─────────────────────────────────────┘
             │ reads
┌────────────┼─────────────────────────────────────┐
│   MCP Server (rmcp, stdio or HTTP transport)    │
│  ┌──────────┐   ┌───────────────────────────┐   │
│  │  Router  │──▶│ tools_query/movement/... │   │
│  └────┬─────┘   └───────────────────────────┘   │
│       │ sends BotCommand (tokio mpsc + oneshot)  │
│       ▼                                           │
│  ┌──────────┐                                    │
│  │ BotEngine│ (azalea client + bevy_ecs)         │
│  └──────────┘                                    │
└──────────────────────────────────────────────────┘
```

## 线程模型

机器人在一个**后台 OS 线程**上运行，拥有自己的 tokio 运行时。egui 桌面 UI
运行在**主线程**上。机器人连接在专用的 OS 线程上启动（而非 `tokio::spawn`），
因为 azalea 的 `ClientBuilder::start` 内部会创建一个 `LocalSet`，而它是
`!Send` 的，无法在多线程运行时上运行。

两侧通过两种机制通信：

- **`Arc<SharedState>`** —— 线程安全的枢纽。世界快照存储在 `ArcSwap` 之后以实现
  **无锁读取**，因此 UI 和 MCP 工具永远不会阻塞机器人。配置使用 `RwLock`；
  在线/连接中/断开标志是 `AtomicBool`；容器句柄和聊天消息使用 `Mutex`
  （全部通过 `unwrap_or_else(|e| e.into_inner())` 实现中毒恢复）。
- **`BotCommand` 通道** —— 一个用于命令的 tokio `mpsc` 通道加上一个用于响应的
  `oneshot` 通道。命令接收器保存在 `ReceiverSlot` 中，并在 `Event::Spawn` 时通过
  `ReceiverLease` 租出；当执行器在 `Event::Disconnect` 时被中止，租约会 drop 并将
  接收器归还给槽位，以供下次重连使用。

`Event::Disconnect` 还会向 ECS 写入 `AppExit::Success`
（`bot.ecs.lock().write_message(AppExit::Success)`），使 `ClientBuilder::start`
返回，连接循环得以重试（azalea 0.15.1 移除了 `Client::exit()`）。一个
`CancellationToken` 会在断开连接时被取消，使重连退避睡眠立即返回，而不是阻塞关闭流程。
