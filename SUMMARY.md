# minecraft-mcp-rs · 审查 TL;DR

> 一屏可读。

## 总体评价

**整体健康度：良好**。AGENTS.md 中点名的所有复杂约束都已落实。主要风险集中在 **测试盲区 + 隐藏 bug** 两块。

## P0（1 项 · 立即修）

- **`patches/rmcp/build.rs` 静默改写开发者 git config**（`core.hooksPath`）。当前 no-op，但任何 `.githooks/` 提交即触发。

## P1（约 20 项 · 短期修）

按修复 ROI 排序：

### 协议与错误
1. `BotError::Offline` 误映射为 `INTERNAL_ERROR`（`error.rs:163`）→ LLM 把"bot 未连"当"系统崩溃"
2. `BotCommandSender::with_timeout` 启动一次性绑定（`main.rs:46`）→ UI 改 `command_timeout_secs` 大多数命令不生效
3. `as_secs()` 截断亚秒超时为 0（`channel.rs:82`）→ 报"after 0s"
4. `drop_item.count == 0` / `attack.entity_id > i32::MAX` 校验在 bot 端而非 MCP 层

### Bot 逻辑
5. **快速重连 `ReceiverLease` 归还竞态**（`events.rs:240`）→ 新连接无执行器，所有命令 `CommandTimeout`
6. `find_standable_neighbor` 把"未加载 chunk"乐观判为"air"（`compound_ops.rs:89`）→ 远处未扫描块被错拒
7. `RealBotClient::goto` 早返回与 `notify` 窗口竞态（`commands.rs:99`）→ 玩家已到达但 MCP 等满超时
8. **`execute_mine_block` 用 `blocks.iter().any` 线性扫描**（`ops.rs:127, 277`）→ 与 M-12 明确要求用 `block_index` 直接冲突
9. `execute_mine_block` `?` 传播 Err，未推进 state machine 到 `Failed`

### 资源与生命周期
10. Disconnect 按钮 `set_online(false)` 提前置位（`settings.rs:193`）→ 假离线窗口
11. `McpServerStatus::Running(addr)` 显示配置地址而非 `listener.local_addr()`
12. `McpServerStatus::Stdio` 在 `serve` 之前就写入（`server.rs:446`）→ 启动失败时短暂显示"运行中"
13. `MinecraftApp::drop` 3 秒 join 是软超时（`app.rs:226-260`）→ 死锁时进程挂起
14. `handle_disconnect` tick abort 与 `AppExit` 写有竞态（`events.rs:281-332`）→ tick 任务在 ECS 拆除后 panic

### 配置 / UI
15. `AppConfig::validate()` 首个错误即返（`config.rs:117`）→ 反复点 Connect
16. `mcp_address` 解析失败静默 fallback 到 127.0.0.1（`main.rs:85-95`）→ 安全期望错位
17. `EditConfig` 仅首帧 lazy-init（`app.rs:279`）→ 未来 hot-reload 时 UI 永不同步

### 测试
18. **CI 不跑 `cargo test` / `cargo clippy`**（`build.yml:68`）→ 违反 AGENTS.md「全过才能交付」
19. 真正超时态零测试（`integration.rs:15` 文档承诺 vs 实际仅测 2 个 `Offline` 分支）
20. **C-2 修复（重连时重新 `set INJECTED_*`）无回归测试** → 静默回归即重连后 bot 注入到错误 SharedState
21. `find_standable_neighbor` 缺 proptest

## 修复路线图

| 优先级 | 工作量 | 内容 |
|--------|--------|------|
| 立即 | 30 分钟 | 删 rmcp `build.rs` + CI 加 test/clippy job |
| 短期 | 1 周 | 修 `as_secs()` 截断 + BotError 重映射 + Sender 动态超时 + 3 项 P1 测试 + 5 个 UI/配置 P1 |
| 中期 | 2 周 | find_standable_neighbor proptest + block_index 替换 + ReceiverLease 重试 + Internal 拆桶 + serve e2e |
| 长期 | - | 去掉 `patches/` 整目录 + 评估 stable toolchain + INJECTED_* 改显式参数 |

## 已确认无问题的核心约束（17 项）

✓ 所有生产 `Mutex::lock` 中毒恢复一致  
✓ `modify_snapshot` 签名 `FnMut` 与 `ArcSwap::rcu` 完全匹配  
✓ 无持锁跨 await  
✓ `INJECTED_*` 重连重新 set + disconnect 清零  
✓ `cancel_token` / `shutdown_token` 独立  
✓ HTTP graceful shutdown `Send + 'static`  
✓ Drop::join 3 秒 timeout  
✓ `BotCommandSender::with_timeout` 装配（但**只装一次**⚠）  
✓ `Offline` vs `CommandTimeout` 分类  
✓ `McpServerStatus` 状态机完整  
✓ HTTP TLS 警告双位置  
✓ BlockUpdate + SectionBlocksUpdate 双重覆盖  
✓ `UseItemWithSlot` 单 dispatch 原子性  
✓ find_standable_neighbor 4×3 Y 优先级  
✓ 全仓库 stdout 零污染  
✓ BotError 17 个变体都映射到 MCP ErrorCode  
✓ CJK 字体降级绝不 panic  

## 不修不行 vs 不修也行

**不修不行**（影响正确性 / 卫生）：
- P0 rmcp `build.rs`（git config 副作用）
- P1.5 ReceiverLease 归还竞态（连接后无执行器）
- P1.18 CI 不跑 test/clippy（组织性风险）
- P1.20 C-2 修复无回归测试（无防护栏）
- P1.1 BotError Offline 误映射（LLM 体验灾难）
- P1.2 Sender 一次性超时（用户配置失效）
- P1.8 execute_mine_block 线性扫描（AGENTS.md 明确要求的 M-12 被违反）

**不修也行**（已加防御 + 触发条件窄）：
- P1.6 find_standable_neighbor 误判（只在 chunk 未加载时）
- P1.7 goto 早返回窗口（azalea 自身问题边界）
- P2 全部（26+ 项性能 / 一致性）
