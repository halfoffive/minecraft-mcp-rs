//! Bot compound operation executor.
//!
//! Orchestrates multi-step operations (mine, place, open, equip) by driving
//! the pure state machines from [`crate::compound_ops`] and issuing
//! [`BotCommand`]s through the command channel.

use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{debug, trace, warn};

use crate::block_data::ItemStack;
use crate::block_data::best_tool_for_block;
use crate::channel::BotCommandSender;
use crate::compound_ops::{
    EquipToolOperation, MineBlockOperation, OpenContainerOperation, OperationEvent, OperationState,
    PlaceBlockOperation,
};
use crate::error::BotError;
use crate::mining_calc::calculate_mine_time;
use crate::state::SharedState;
use crate::tool_select::{find_tool_in_inventory, select_tool_for_block};
use crate::types::{BlockPos, BotCommand, BotResult, MaterialTier, ToolType};

// ---------------------------------------------------------------------------
// Type-conversion helpers
// ---------------------------------------------------------------------------
//
// The crate previously had duplicate `BlockPos`/`ToolType`/`MaterialTier`
// definitions in `error.rs` and `types.rs`. Phase 4 unified them: `error.rs`
// now re-exports from `types.rs`, so no conversion is needed anymore. The
// `BotError` variants accept `types::BlockPos` / `types::ToolType` directly.

// ---------------------------------------------------------------------------
// CompoundOpExecutor
// ---------------------------------------------------------------------------

/// High-level executor for compound bot operations.
///
/// Each method drives a state machine from [`crate::compound_ops`] by
/// translating states into [`BotCommand`]s sent through the command channel,
/// waiting for responses, and advancing the machine.
#[derive(Debug, Clone)]
pub struct CompoundOpExecutor {
    sender: BotCommandSender,
    state: Arc<SharedState>,
}

impl CompoundOpExecutor {
    /// Create a new executor bound to a command sender and shared state.
    pub fn new(sender: BotCommandSender, state: Arc<SharedState>) -> Self {
        Self { sender, state }
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    /// Query the bot's inventory by sending [`BotCommand::QueryInventory`].
    async fn query_inventory(&self) -> Result<Vec<Option<ItemStack>>, BotError> {
        let result = self.sender.send_command(BotCommand::QueryInventory).await?;
        let data = result.data.unwrap_or(serde_json::Value::Null);

        if let Some(arr) = data.as_array() {
            let inventory: Vec<Option<ItemStack>> = arr
                .iter()
                .map(|item| {
                    if item.is_null() {
                        None
                    } else {
                        let item_id = item.get("item_id")?.as_str()?.to_string();
                        let count = item.get("count")?.as_u64()? as u8;
                        Some(ItemStack { item_id, count })
                    }
                })
                .collect();
            Ok(inventory)
        } else {
            Ok(vec![])
        }
    }

    // -----------------------------------------------------------------------
    // execute_mine_block
    // -----------------------------------------------------------------------

    /// Mine a block at the given position.
    ///
    /// # Steps
    /// 1. Check bot is online.
    /// 2. Query block type from the world snapshot.
    /// 3. If `use_best_tool`, select the best tool for the block.
    /// 4. If a specialised tool is required but not found, return [`BotError::ToolNotFound`].
    /// 5. If the tool is not in the hotbar, switch to its slot.
    /// 6. Walk to the block vicinity.
    /// 7. Verify arrival.
    /// 8. Start mining.
    /// 9. Wait for mining completion (sleep calculated from [`mining_calc`](crate::mining_calc)).
    /// 10. Verify the block is broken.
    /// 11. Return success or failure.
    pub async fn execute_mine_block(
        &self,
        pos: BlockPos,
        use_best_tool: bool,
    ) -> Result<BotResult, BotError> {
        trace!(?pos, use_best_tool, "execute_mine_block start");

        // Step 1: Check online
        if !self.state.is_online() {
            warn!("bot offline, cannot mine block");
            return Err(BotError::Offline("bot is not connected".into()));
        }

        // Step 2: Query block type
        let snapshot = self.state.read_snapshot();
        let block_type = snapshot
            .blocks
            .iter()
            .find(|b| b.position == pos)
            .map(|b| b.block_type.clone())
            .ok_or_else(|| {
                warn!(?pos, "block not found in snapshot");
                BotError::BlockNotFound(pos)
            })?;

        trace!(?pos, %block_type, "found block in snapshot");

        // Steps 3-5: Tool selection and equipping
        let required_tool = best_tool_for_block(&block_type);
        let mut tool_type = ToolType::Hand;
        let mut material = MaterialTier::Wood;

        if use_best_tool && required_tool != ToolType::Hand {
            let inventory = self.query_inventory().await?;
            let selection = select_tool_for_block(&block_type, &inventory);

            // Step 4: Tool needed but not in inventory
            if selection.tool_type == ToolType::Hand {
                return Err(BotError::ToolNotFound {
                    tool_type: required_tool,
                    material: None,
                });
            }

            tool_type = selection.tool_type;
            material = selection.material.unwrap_or(MaterialTier::Wood);

            // Step 5: Switch to tool slot if we have one
            if let Some(slot) = selection.hotbar_slot {
                trace!(slot, "switching to tool slot");
                self.sender
                    .send_command(BotCommand::SwitchHotbarSlot(slot))
                    .await?;
            }
        }

        // Build state machine
        let op = MineBlockOperation::new(pos, tool_type);
        let mut state = OperationState::Idle;

        // Step 6-10: Drive state machine
        state = op.advance(state, OperationEvent::Start);

        while !matches!(state, OperationState::Completed | OperationState::Failed(_)) {
            match op.current_action(&state) {
                Some(BotCommand::MoveTo(target)) => {
                    trace!(?target, "sending MoveTo");
                    let result = self.sender.send_command(BotCommand::MoveTo(target)).await?;
                    if !result.success {
                        state = op.advance(
                            state,
                            OperationEvent::Failed(BotError::PathfindingFailed {
                                target,
                                reason: result.message,
                            }),
                        );
                        continue;
                    }
                    state = op.advance(state, OperationEvent::Arrived);
                }
                Some(BotCommand::EquipTool(t)) => {
                    trace!(?t, "sending EquipTool");
                    let result = self.sender.send_command(BotCommand::EquipTool(t)).await?;
                    if !result.success {
                        state = op.advance(
                            state,
                            OperationEvent::Failed(BotError::ToolNotFound {
                                tool_type: t,
                                material: None,
                            }),
                        );
                        continue;
                    }
                    state = op.advance(state, OperationEvent::ToolEquipped);
                }
                Some(BotCommand::BreakBlock(bp)) => {
                    trace!(?bp, "sending BreakBlock");
                    let result = self.sender.send_command(BotCommand::BreakBlock(bp)).await?;
                    if !result.success {
                        state = op.advance(
                            state,
                            OperationEvent::Failed(BotError::MiningInterrupted {
                                reason: result.message,
                            }),
                        );
                        continue;
                    }

                    // Advance to WaitingForResult (ExecutingAction + ActionStarted → WaitingForResult)
                    state = op.advance(state, OperationEvent::ActionStarted);

                    // Step 9: Wait for mining completion.
                    let mine_time = calculate_mine_time(&block_type, tool_type, material);
                    trace!(mine_time, "waiting for mining completion");

                    // Unbreakable blocks (e.g. bedrock) yield INFINITY, which
                    // would panic `Duration::from_secs_f64`. Fail fast with a
                    // clear error instead of crashing the bot thread.
                    if !mine_time.is_finite() {
                        state = op.advance(
                            state,
                            OperationEvent::Failed(BotError::MiningInterrupted {
                                reason: format!(
                                    "block {block_type} is unbreakable (infinite mine time)"
                                ),
                            }),
                        );
                        continue;
                    }
                    sleep(Duration::from_secs_f64(mine_time)).await;

                    // Step 10: Verify block broken
                    let new_snapshot = self.state.read_snapshot();
                    let still_there = new_snapshot.blocks.iter().any(|b| b.position == pos);
                    if still_there {
                        warn!(?pos, "block still present after mining time");
                        state = op.advance(
                            state,
                            OperationEvent::Failed(BotError::MiningInterrupted {
                                reason: "block still present after mining time".into(),
                            }),
                        );
                    } else {
                        state = op.advance(state, OperationEvent::BlockBroken);
                    }
                }
                _ => {
                    // No action for this state — should be terminal or invalid
                    break;
                }
            }
        }

        match state {
            OperationState::Completed => {
                debug!(?pos, %block_type, "mine block completed");
                Ok(BotResult {
                    success: true,
                    message: format!("Mined {} at {}", block_type, pos),
                    data: None,
                })
            }
            OperationState::Failed(err) => Err(err),
            other => {
                warn!(?other, "mine block ended in non-terminal state");
                Err(BotError::Internal(format!(
                    "mine block ended in state {:?}",
                    other
                )))
            }
        }
    }

    // -----------------------------------------------------------------------
    // execute_place_block
    // -----------------------------------------------------------------------

    /// Place a block at the given position.
    ///
    /// # Steps
    /// 1. Find the item in the inventory.
    /// 2. Select it in the hotbar.
    /// 3. Walk near the target.
    /// 4. Place the block.
    /// 5. Verify the block was placed.
    pub async fn execute_place_block(
        &self,
        pos: BlockPos,
        block_type: String,
    ) -> Result<BotResult, BotError> {
        trace!(?pos, %block_type, "execute_place_block start");

        if !self.state.is_online() {
            return Err(BotError::Offline("bot is not connected".into()));
        }

        // Step 1: Find item in inventory
        let inventory = self.query_inventory().await?;
        let has_item = inventory
            .iter()
            .any(|slot| slot.as_ref().is_some_and(|item| item.item_id == block_type));

        if !has_item {
            return Err(BotError::ToolNotFound {
                tool_type: ToolType::Hand, // generic fallback (no block item)
                material: None,
            });
        }

        // Step 2: Select in hotbar. Only hotbar slots (0-8) can be selected
        // directly; an item in the main inventory (slot 9-35) can't be
        // switched to without an inventory-move flow, so surface a clear
        // error instead of letting the executor reject slot >= 9.
        let slot = inventory
            .iter()
            .position(|s| s.as_ref().is_some_and(|item| item.item_id == block_type))
            .map(|i| i as u8);

        if let Some(s) = slot {
            if s > 8 {
                return Err(BotError::Internal(format!(
                    "{block_type} is in main inventory slot {s}; move it to a hotbar slot (0-8) before placing"
                )));
            }
            self.sender
                .send_command(BotCommand::SwitchHotbarSlot(s))
                .await?;
        }

        // Build state machine
        let op = PlaceBlockOperation::new(pos, block_type.clone());
        let mut state = OperationState::Idle;

        state = op.advance(state, OperationEvent::Start);
        // EquippingTool has no current_action — item selection was handled above,
        // so advance past it with ToolEquipped.
        state = op.advance(state, OperationEvent::ToolEquipped);

        while !matches!(state, OperationState::Completed | OperationState::Failed(_)) {
            match op.current_action(&state) {
                Some(BotCommand::MoveTo(target)) => {
                    let result = self.sender.send_command(BotCommand::MoveTo(target)).await?;
                    if !result.success {
                        state = op.advance(
                            state,
                            OperationEvent::Failed(BotError::PathfindingFailed {
                                target,
                                reason: result.message,
                            }),
                        );
                        continue;
                    }
                    state = op.advance(state, OperationEvent::Arrived);
                }
                Some(BotCommand::PlaceBlock(target, bt)) => {
                    let result = self
                        .sender
                        .send_command(BotCommand::PlaceBlock(target, bt))
                        .await?;
                    if !result.success {
                        state = op.advance(
                            state,
                            OperationEvent::Failed(BotError::Internal(result.message)),
                        );
                        continue;
                    }

                    // Verify block placed
                    sleep(Duration::from_millis(200)).await;
                    let new_snapshot = self.state.read_snapshot();
                    let placed = new_snapshot
                        .blocks
                        .iter()
                        .any(|b| b.position == pos && b.block_type == block_type);
                    if placed {
                        state = op.advance(state, OperationEvent::BlockPlaced);
                    } else {
                        state = op.advance(
                            state,
                            OperationEvent::Failed(BotError::Internal("block not placed".into())),
                        );
                    }
                }
                _ => break,
            }
        }

        match state {
            OperationState::Completed => Ok(BotResult {
                success: true,
                message: format!("Placed {} at {}", block_type, pos),
                data: None,
            }),
            OperationState::Failed(err) => Err(err),
            other => Err(BotError::Internal(format!(
                "place block ended in state {:?}",
                other
            ))),
        }
    }

    // -----------------------------------------------------------------------
    // execute_open_container
    // -----------------------------------------------------------------------

    /// Open a container at the given position.
    ///
    /// # Steps
    /// 1. Walk near the container.
    /// 2. Send `OpenContainer` command.
    /// 3. Return success (container handle storage is handled by the lower-level
    ///    command handler).
    pub async fn execute_open_container(&self, pos: BlockPos) -> Result<BotResult, BotError> {
        trace!(?pos, "execute_open_container start");

        if !self.state.is_online() {
            return Err(BotError::Offline("bot is not connected".into()));
        }

        let op = OpenContainerOperation::new(pos);
        let mut state = OperationState::Idle;

        state = op.advance(state, OperationEvent::Start);

        while !matches!(state, OperationState::Completed | OperationState::Failed(_)) {
            match op.current_action(&state) {
                Some(BotCommand::MoveTo(target)) => {
                    let result = self.sender.send_command(BotCommand::MoveTo(target)).await?;
                    if !result.success {
                        state = op.advance(
                            state,
                            OperationEvent::Failed(BotError::PathfindingFailed {
                                target,
                                reason: result.message,
                            }),
                        );
                        continue;
                    }
                    state = op.advance(state, OperationEvent::Arrived);
                }
                Some(BotCommand::OpenContainer(target)) => {
                    let result = self
                        .sender
                        .send_command(BotCommand::OpenContainer(target))
                        .await?;
                    if !result.success {
                        state =
                            op.advance(state, OperationEvent::Failed(BotError::ContainerTimeout));
                        continue;
                    }
                    state = op.advance(state, OperationEvent::ContainerOpened);
                }
                _ => break,
            }
        }

        match state {
            OperationState::Completed => {
                debug!(?pos, "open container completed");
                Ok(BotResult {
                    success: true,
                    message: format!("Opened container at {}", pos),
                    data: None,
                })
            }
            OperationState::Failed(err) => Err(err),
            other => Err(BotError::Internal(format!(
                "open container ended in state {:?}",
                other
            ))),
        }
    }

    // -----------------------------------------------------------------------
    // execute_equip_tool
    // -----------------------------------------------------------------------

    /// Equip the best available tool of the given type.
    ///
    /// # Steps
    /// 1. Find the best tool in the inventory.
    /// 2. Move to hotbar if needed (by switching to the slot).
    /// 3. Drive the `EquipToolOperation` state machine.
    /// 4. Return success.
    pub async fn execute_equip_tool(&self, tool_type: ToolType) -> Result<BotResult, BotError> {
        trace!(?tool_type, "execute_equip_tool start");

        if !self.state.is_online() {
            return Err(BotError::Offline("bot is not connected".into()));
        }

        // Step 1: Find best tool
        let inventory = self.query_inventory().await?;
        let found = find_tool_in_inventory(&tool_type, &inventory);

        if found.is_none() && tool_type != ToolType::Hand {
            return Err(BotError::ToolNotFound {
                tool_type,
                material: None,
            });
        }

        // 装备空手：无需切换槽位，直接返回成功
        if found.is_none() && tool_type == ToolType::Hand {
            return Ok(BotResult {
                success: true,
                message: "Equipped Hand (no slot switch needed)".to_string(),
                data: None,
            });
        }

        let (_material, slot) = found.unwrap_or((MaterialTier::Wood, 0));

        // Step 2: Switch to the slot
        self.sender
            .send_command(BotCommand::SwitchHotbarSlot(slot))
            .await?;

        // Step 3: Drive state machine
        let op = EquipToolOperation::new(tool_type);
        let mut state = OperationState::Idle;

        state = op.advance(state, OperationEvent::Start);

        while !matches!(state, OperationState::Completed | OperationState::Failed(_)) {
            match op.current_action(&state) {
                Some(BotCommand::EquipTool(t)) => {
                    let result = self.sender.send_command(BotCommand::EquipTool(t)).await?;
                    if !result.success {
                        state = op.advance(
                            state,
                            OperationEvent::Failed(BotError::ToolNotFound {
                                tool_type: t,
                                material: None,
                            }),
                        );
                        continue;
                    }
                    state = op.advance(state, OperationEvent::ToolEquipped);
                }
                _ => break,
            }
        }

        match state {
            OperationState::Completed => Ok(BotResult {
                success: true,
                message: format!("Equipped {:?} in slot {}", tool_type, slot),
                data: None,
            }),
            OperationState::Failed(err) => Err(err),
            other => Err(BotError::Internal(format!(
                "equip tool ended in state {:?}",
                other
            ))),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::create_command_channel;
    use crate::config::AppConfig;
    use crate::types::{BlockEntry, GameMode, SelfPlayer, WorldSnapshot};

    // ── Helpers ───────────────────────────────────────────────────────────

    fn make_snapshot_with_block(pos: BlockPos, block_type: &str) -> WorldSnapshot {
        WorldSnapshot {
            blocks: vec![BlockEntry {
                position: pos,
                block_type: block_type.into(),
                block_state: None,
            }],
            entities: vec![],
            self_player: SelfPlayer {
                uuid: "test".into(),
                username: "TestBot".into(),
                position: BlockPos::new(0, 64, 0),
                health: 20.0,
                hunger: 20,
                gamemode: GameMode::Survival,
                held_item_slot: 0,
                inventory: Vec::new(),
            },
            timestamp: 1,
            chunk_summary: vec![],
            commands_enabled: None,
        }
    }

    fn make_empty_snapshot() -> WorldSnapshot {
        WorldSnapshot {
            blocks: vec![],
            entities: vec![],
            self_player: SelfPlayer {
                uuid: "test".into(),
                username: "TestBot".into(),
                position: BlockPos::new(0, 64, 0),
                health: 20.0,
                hunger: 20,
                gamemode: GameMode::Survival,
                held_item_slot: 0,
                inventory: Vec::new(),
            },
            timestamp: 1,
            chunk_summary: vec![],
            commands_enabled: None,
        }
    }

    fn inventory_json(items: &[Option<ItemStack>]) -> serde_json::Value {
        let arr: Vec<serde_json::Value> = items
            .iter()
            .map(|slot| match slot {
                Some(item) => serde_json::json!({
                    "item_id": item.item_id,
                    "count": item.count,
                }),
                None => serde_json::Value::Null,
            })
            .collect();
        serde_json::Value::Array(arr)
    }

    /// Spawn a mock responder that replies to commands and optionally mutates
    /// the shared snapshot.
    fn spawn_mock_responder(
        mut receiver: crate::channel::BotCommandReceiver,
        inventory: Vec<Option<ItemStack>>,
        mut snapshot: WorldSnapshot,
        state: Arc<SharedState>,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            while let Some(wrapped) = receiver.recv().await {
                let result = match &wrapped.command {
                    BotCommand::QueryInventory => Ok(BotResult {
                        success: true,
                        message: "inventory".into(),
                        data: Some(inventory_json(&inventory)),
                    }),
                    BotCommand::MoveTo(pos) => {
                        snapshot.self_player.position = *pos;
                        state.update_snapshot(snapshot.clone());
                        Ok(BotResult {
                            success: true,
                            message: "moved".into(),
                            data: None,
                        })
                    }
                    BotCommand::BreakBlock(pos) => {
                        snapshot.blocks.retain(|b| b.position != *pos);
                        state.update_snapshot(snapshot.clone());
                        Ok(BotResult {
                            success: true,
                            message: "mining started".into(),
                            data: None,
                        })
                    }
                    BotCommand::PlaceBlock(pos, bt) => {
                        snapshot.blocks.push(BlockEntry {
                            position: *pos,
                            block_type: bt.clone(),
                            block_state: None,
                        });
                        state.update_snapshot(snapshot.clone());
                        Ok(BotResult {
                            success: true,
                            message: "placed".into(),
                            data: None,
                        })
                    }
                    BotCommand::OpenContainer(_) => Ok(BotResult {
                        success: true,
                        message: "opened".into(),
                        data: None,
                    }),
                    BotCommand::SwitchHotbarSlot(slot) => {
                        snapshot.self_player.held_item_slot = *slot;
                        state.update_snapshot(snapshot.clone());
                        Ok(BotResult {
                            success: true,
                            message: "switched".into(),
                            data: None,
                        })
                    }
                    BotCommand::EquipTool(tool) => Ok(BotResult {
                        success: true,
                        message: format!("equipped {:?}", tool),
                        data: None,
                    }),
                    _ => Ok(BotResult {
                        success: true,
                        message: "ok".into(),
                        data: None,
                    }),
                };
                let _ = wrapped.respond_to.send(result);
            }
        })
    }

    fn setup(
        inventory: Vec<Option<ItemStack>>,
        snapshot: WorldSnapshot,
    ) -> (
        CompoundOpExecutor,
        tokio::task::JoinHandle<()>,
        Arc<SharedState>,
    ) {
        let (sender, receiver) = create_command_channel(10);
        let state = Arc::new(SharedState::new(AppConfig::default()));
        state.update_snapshot(snapshot.clone());
        state.set_online(true);
        let executor = CompoundOpExecutor::new(sender, Arc::clone(&state));
        let handle = spawn_mock_responder(receiver, inventory, snapshot, Arc::clone(&state));
        (executor, handle, state)
    }

    // ── execute_mine_block: offline ───────────────────────────────────────

    #[tokio::test]
    async fn test_mine_block_offline() {
        let (sender, _receiver) = create_command_channel(10);
        let state = Arc::new(SharedState::new(AppConfig::default()));
        state.set_online(false);
        let executor = CompoundOpExecutor::new(sender, state);

        let pos = BlockPos::new(10, 64, 20);
        let result = executor.execute_mine_block(pos, false).await;

        assert!(result.is_err());
        assert!(matches!(result, Err(BotError::Offline(_))));
    }

    // ── execute_mine_block: block not found ───────────────────────────────

    #[tokio::test]
    async fn test_mine_block_not_found() {
        let (executor, _handle, _state) = setup(vec![], make_empty_snapshot());

        let pos = BlockPos::new(10, 64, 20);
        let result = executor.execute_mine_block(pos, false).await;

        assert!(result.is_err());
        assert!(matches!(result, Err(BotError::BlockNotFound(_))));
    }

    // ── execute_mine_block: happy path (hand, no tool needed) ─────────────

    #[tokio::test]
    async fn test_mine_block_happy_path_hand() {
        let pos = BlockPos::new(10, 64, 20);
        let snapshot = make_snapshot_with_block(pos, "dirt");
        let (executor, _handle, _state) = setup(vec![], snapshot);

        let result = executor.execute_mine_block(pos, false).await;

        assert!(result.is_ok());
        let bot_result = result.unwrap();
        assert!(bot_result.success);
        assert!(bot_result.message.contains("Mined dirt"));
    }

    // ── execute_mine_block: with best tool ────────────────────────────────

    #[tokio::test]
    async fn test_mine_block_with_best_tool() {
        let pos = BlockPos::new(10, 64, 20);
        let snapshot = make_snapshot_with_block(pos, "stone");
        let inventory = vec![
            None,
            Some(ItemStack {
                item_id: "iron_pickaxe".into(),
                count: 1,
            }),
        ];
        let (executor, _handle, state) = setup(inventory, snapshot);

        let result = executor.execute_mine_block(pos, true).await;

        assert!(result.is_ok(), "expected success, got: {:?}", result);
        let bot_result = result.unwrap();
        assert!(bot_result.success);
        assert!(bot_result.message.contains("Mined stone"));

        // Verify the block was removed from snapshot
        let final_snapshot = state.read_snapshot();
        assert!(!final_snapshot.blocks.iter().any(|b| b.position == pos));
    }

    // ── execute_mine_block: tool not found ────────────────────────────────

    #[tokio::test]
    async fn test_mine_block_tool_not_found() {
        let pos = BlockPos::new(10, 64, 20);
        let snapshot = make_snapshot_with_block(pos, "stone");
        let (executor, _handle, _state) = setup(vec![], snapshot);

        let result = executor.execute_mine_block(pos, true).await;

        assert!(result.is_err());
        assert!(matches!(result, Err(BotError::ToolNotFound { .. })));
    }

    // ── execute_mine_block: mining interrupted (BreakBlock fails) ───────

    #[tokio::test]
    async fn test_mine_block_mining_interrupted() {
        let pos = BlockPos::new(10, 64, 20);
        let snapshot = make_snapshot_with_block(pos, "obsidian");
        let inventory = vec![Some(ItemStack {
            item_id: "diamond_pickaxe".into(),
            count: 1,
        })];
        let (sender, mut receiver) = create_command_channel(10);
        let state = Arc::new(SharedState::new(AppConfig::default()));
        state.update_snapshot(snapshot.clone());
        state.set_online(true);
        let executor = CompoundOpExecutor::new(sender, Arc::clone(&state));

        // Custom mock: BreakBlock returns failure to simulate mining interruption.
        let responder = tokio::spawn(async move {
            while let Some(wrapped) = receiver.recv().await {
                let result = match &wrapped.command {
                    BotCommand::QueryInventory => Ok(BotResult {
                        success: true,
                        message: "inventory".into(),
                        data: Some(inventory_json(&inventory)),
                    }),
                    BotCommand::MoveTo(target) => {
                        let snap = (*state.read_snapshot()).clone();
                        state.update_snapshot(WorldSnapshot {
                            self_player: SelfPlayer {
                                position: *target,
                                ..snap.self_player.clone()
                            },
                            ..snap
                        });
                        Ok(BotResult {
                            success: true,
                            message: "moved".into(),
                            data: None,
                        })
                    }
                    BotCommand::BreakBlock(_) => Ok(BotResult {
                        success: false,
                        message: "mining interrupted by creeper".into(),
                        data: None,
                    }),
                    BotCommand::SwitchHotbarSlot(_) => Ok(BotResult {
                        success: true,
                        message: "switched".into(),
                        data: None,
                    }),
                    BotCommand::EquipTool(_) => Ok(BotResult {
                        success: true,
                        message: "equipped".into(),
                        data: None,
                    }),
                    _ => Ok(BotResult {
                        success: true,
                        message: "ok".into(),
                        data: None,
                    }),
                };
                let _ = wrapped.respond_to.send(result);
            }
        });

        let result = executor.execute_mine_block(pos, true).await;

        // Drop executor so the sender channel closes, allowing the mock to exit.
        drop(executor);

        assert!(result.is_err());
        assert!(matches!(result, Err(BotError::MiningInterrupted { .. })));

        responder.await.unwrap();
    }

    // ── execute_place_block: offline ──────────────────────────────────────

    #[tokio::test]
    async fn test_place_block_offline() {
        let (sender, _receiver) = create_command_channel(10);
        let state = Arc::new(SharedState::new(AppConfig::default()));
        state.set_online(false);
        let executor = CompoundOpExecutor::new(sender, state);

        let pos = BlockPos::new(10, 64, 20);
        let result = executor.execute_place_block(pos, "stone".into()).await;

        assert!(result.is_err());
        assert!(matches!(result, Err(BotError::Offline(_))));
    }

    // ── execute_place_block: no item ─────────────────────────────────────

    #[tokio::test]
    async fn test_place_block_no_item() {
        let pos = BlockPos::new(10, 64, 20);
        let snapshot = make_empty_snapshot();
        let (executor, _handle, _state) = setup(vec![], snapshot);

        let result = executor.execute_place_block(pos, "stone".into()).await;

        assert!(result.is_err());
        assert!(matches!(result, Err(BotError::ToolNotFound { .. })));
    }

    // ── execute_place_block: happy path ─────────────────────────────────

    #[tokio::test]
    async fn test_place_block_happy_path() {
        let pos = BlockPos::new(10, 64, 20);
        let snapshot = make_empty_snapshot();
        let inventory = vec![Some(ItemStack {
            item_id: "stone".into(),
            count: 64,
        })];
        let (executor, _handle, state) = setup(inventory, snapshot);

        let result = executor.execute_place_block(pos, "stone".into()).await;

        assert!(result.is_ok(), "expected success, got: {:?}", result);
        let bot_result = result.unwrap();
        assert!(bot_result.success);
        assert!(bot_result.message.contains("Placed stone"));

        // Verify the block was added to snapshot
        let final_snapshot = state.read_snapshot();
        assert!(
            final_snapshot
                .blocks
                .iter()
                .any(|b| b.position == pos && b.block_type == "stone")
        );
    }

    // ── execute_open_container: offline ───────────────────────────────────

    #[tokio::test]
    async fn test_open_container_offline() {
        let (sender, _receiver) = create_command_channel(10);
        let state = Arc::new(SharedState::new(AppConfig::default()));
        state.set_online(false);
        let executor = CompoundOpExecutor::new(sender, state);

        let pos = BlockPos::new(10, 64, 20);
        let result = executor.execute_open_container(pos).await;

        assert!(result.is_err());
        assert!(matches!(result, Err(BotError::Offline(_))));
    }

    // ── execute_open_container: happy path ──────────────────────────────

    #[tokio::test]
    async fn test_open_container_happy_path() {
        let pos = BlockPos::new(10, 64, 20);
        let snapshot = make_empty_snapshot();
        let (executor, _handle, _state) = setup(vec![], snapshot);

        let result = executor.execute_open_container(pos).await;

        assert!(result.is_ok(), "expected success, got: {:?}", result);
        let bot_result = result.unwrap();
        assert!(bot_result.success);
        assert!(bot_result.message.contains("Opened container"));
    }

    // ── execute_equip_tool: offline ─────────────────────────────────────

    #[tokio::test]
    async fn test_equip_tool_offline() {
        let (sender, _receiver) = create_command_channel(10);
        let state = Arc::new(SharedState::new(AppConfig::default()));
        state.set_online(false);
        let executor = CompoundOpExecutor::new(sender, state);

        let result = executor.execute_equip_tool(ToolType::Pickaxe).await;

        assert!(result.is_err());
        assert!(matches!(result, Err(BotError::Offline(_))));
    }

    // ── execute_equip_tool: not found ───────────────────────────────────

    #[tokio::test]
    async fn test_equip_tool_not_found() {
        let snapshot = make_empty_snapshot();
        let (executor, _handle, _state) = setup(vec![], snapshot);

        let result = executor.execute_equip_tool(ToolType::Pickaxe).await;

        assert!(result.is_err());
        assert!(matches!(result, Err(BotError::ToolNotFound { .. })));
    }

    // ── execute_equip_tool: happy path ──────────────────────────────────

    #[tokio::test]
    async fn test_equip_tool_happy_path() {
        let snapshot = make_empty_snapshot();
        let inventory = vec![Some(ItemStack {
            item_id: "diamond_pickaxe".into(),
            count: 1,
        })];
        let (executor, _handle, state) = setup(inventory, snapshot);

        let result = executor.execute_equip_tool(ToolType::Pickaxe).await;

        assert!(result.is_ok(), "expected success, got: {:?}", result);
        let bot_result = result.unwrap();
        assert!(bot_result.success);
        assert!(bot_result.message.contains("Equipped Pickaxe"));

        // Verify held slot was updated
        let final_snapshot = state.read_snapshot();
        assert_eq!(final_snapshot.self_player.held_item_slot, 0);
    }

    // ── execute_equip_tool: selects_best_tier ─────────────────────────────

    #[tokio::test]
    async fn test_equip_tool_selects_best_tier() {
        let snapshot = make_empty_snapshot();
        let inventory = vec![
            Some(ItemStack {
                item_id: "wooden_pickaxe".into(),
                count: 1,
            }),
            Some(ItemStack {
                item_id: "iron_pickaxe".into(),
                count: 1,
            }),
        ];
        let (executor, _handle, state) = setup(inventory, snapshot);

        let result = executor.execute_equip_tool(ToolType::Pickaxe).await;

        assert!(result.is_ok(), "expected success, got: {:?}", result);
        // iron_pickaxe is at slot 1, so held_item_slot should be 1
        let final_snapshot = state.read_snapshot();
        assert_eq!(final_snapshot.self_player.held_item_slot, 1);
    }

    // ── execute_equip_tool: Hand with no tool does not switch slot ───────

    #[tokio::test]
    async fn test_equip_tool_hand_no_switch() {
        // Use a non-zero held_item_slot to detect any SwitchHotbarSlot(0).
        let mut snapshot = make_empty_snapshot();
        snapshot.self_player.held_item_slot = 3;
        let (executor, _handle, state) = setup(vec![], snapshot);

        let result = executor.execute_equip_tool(ToolType::Hand).await;

        assert!(result.is_ok(), "expected success, got: {:?}", result);
        let bot_result = result.unwrap();
        assert!(bot_result.success);
        assert!(bot_result.message.contains("Equipped Hand"));

        // Verify held slot was NOT changed (no SwitchHotbarSlot sent)
        let final_snapshot = state.read_snapshot();
        assert_eq!(final_snapshot.self_player.held_item_slot, 3);
    }

    // ── Failure recovery: pathfinding fails during mine ─────────────────

    #[tokio::test]
    async fn test_mine_block_pathfinding_fails() {
        let pos = BlockPos::new(10, 64, 20);
        let snapshot = make_snapshot_with_block(pos, "stone");
        let (sender, mut receiver) = create_command_channel(10);
        let state = Arc::new(SharedState::new(AppConfig::default()));
        state.update_snapshot(snapshot.clone());
        state.set_online(true);
        let executor = CompoundOpExecutor::new(sender, Arc::clone(&state));

        // Spawn a responder that fails MoveTo
        let responder = tokio::spawn(async move {
            while let Some(wrapped) = receiver.recv().await {
                let result = match &wrapped.command {
                    BotCommand::QueryInventory => Ok(BotResult {
                        success: true,
                        message: "inventory".into(),
                        data: Some(inventory_json(&[])),
                    }),
                    BotCommand::MoveTo(_) => Ok(BotResult {
                        success: false,
                        message: "path blocked".into(),
                        data: None,
                    }),
                    _ => Ok(BotResult {
                        success: true,
                        message: "ok".into(),
                        data: None,
                    }),
                };
                let _ = wrapped.respond_to.send(result);
            }
        });

        let result = executor.execute_mine_block(pos, false).await;

        // Drop executor so the sender channel closes, allowing the mock to exit.
        drop(executor);

        assert!(result.is_err());
        assert!(matches!(result, Err(BotError::PathfindingFailed { .. })));

        responder.await.unwrap();
    }

    // ── CompoundOpExecutor construction ───────────────────────────────────

    #[test]
    fn test_executor_new() {
        let (sender, _receiver) = create_command_channel(10);
        let state = Arc::new(SharedState::new(AppConfig::default()));
        let executor = CompoundOpExecutor::new(sender, state);
        // Just verify it constructs without panic
        let _ = executor;
    }

    // ── State machine integration: mine block reaches all states ─────────

    #[tokio::test]
    async fn test_mine_block_state_machine_reaches_all_states() {
        let pos = BlockPos::new(5, 64, 5);
        let snapshot = make_snapshot_with_block(pos, "dirt");
        let (executor, _handle, _state) = setup(vec![], snapshot);

        let result = executor.execute_mine_block(pos, false).await;
        assert!(result.is_ok());
    }

    // ── State machine integration: place block reaches all states ─────────

    #[tokio::test]
    async fn test_place_block_state_machine_reaches_all_states() {
        let pos = BlockPos::new(5, 64, 5);
        let snapshot = make_empty_snapshot();
        let inventory = vec![Some(ItemStack {
            item_id: "oak_planks".into(),
            count: 10,
        })];
        let (executor, _handle, _state) = setup(inventory, snapshot);

        let result = executor.execute_place_block(pos, "oak_planks".into()).await;
        assert!(result.is_ok());
    }

    // ── State machine integration: open container reaches all states ──────

    #[tokio::test]
    async fn test_open_container_state_machine_reaches_all_states() {
        let pos = BlockPos::new(5, 64, 5);
        let snapshot = make_empty_snapshot();
        let (executor, _handle, _state) = setup(vec![], snapshot);

        let result = executor.execute_open_container(pos).await;
        assert!(result.is_ok());
    }

    // ── State machine integration: equip tool reaches all states ─────────

    #[tokio::test]
    async fn test_equip_tool_state_machine_reaches_all_states() {
        let snapshot = make_empty_snapshot();
        let inventory = vec![Some(ItemStack {
            item_id: "stone_axe".into(),
            count: 1,
        })];
        let (executor, _handle, _state) = setup(inventory, snapshot);

        let result = executor.execute_equip_tool(ToolType::Axe).await;
        assert!(result.is_ok());
    }
}
