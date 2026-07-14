//! Bot compound operation executor.
//!
//! Orchestrates multi-step operations (mine, place, open, equip) by driving
//! the pure state machines from [`crate::compound_ops`] and dispatching
//! [`BotCommand`]s directly through [`CommandExecutor::dispatch`].
//!
//! Sub-commands are dispatched via a `&CommandExecutor` reference rather than
//! through the [`crate::channel::BotCommandSender`] channel. The channel's
//! only consumer is [`CommandExecutor::run_with_lease`], which processes one
//! command at a time — so a compound operation that sends sub-commands
//! through the same channel would block forever waiting for a consumer that
//! is already awaiting the outer `dispatch` call (re-entrant deadlock).
//! Dispatching directly via `&CommandExecutor` bypasses the channel entirely:
//! sub-command handlers run inline on the same call stack.

use std::time::Duration;
use tokio::time::sleep;
use tracing::{debug, trace, warn};

use crate::block_data::ItemStack;
use crate::block_data::best_tool_for_block;
use crate::bot::commands::{BotActions, CommandExecutor};
use crate::compound_ops::{
    EquipToolOperation, MineBlockOperation, OpenContainerOperation, OperationEvent, OperationState,
    PlaceBlockOperation,
};
use crate::error::BotError;
use crate::mining_calc::calculate_mine_time;
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
/// translating states into [`BotCommand`]s dispatched directly through
/// [`CommandExecutor::dispatch`], and advancing the machine based on the
/// results.
///
/// All methods are associated functions taking `&CommandExecutor<B>` as the
/// first parameter. The executor holds no state of its own — it is a unit
/// struct that exists only as a namespace for the compound-operation logic.
/// Sub-commands are dispatched via the `&CommandExecutor` reference (not the
/// [`crate::channel::BotCommandSender`] channel) to avoid re-entrant
/// deadlock when the executor is already inside `run_with_lease` consuming
/// the outer command.
#[derive(Debug, Clone)]
pub struct CompoundOpExecutor;

impl CompoundOpExecutor {
    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    /// Query the bot's inventory by dispatching [`BotCommand::QueryInventory`].
    async fn query_inventory<B: BotActions>(
        executor: &CommandExecutor<B>,
    ) -> Result<Vec<Option<ItemStack>>, BotError> {
        let result = executor.dispatch(BotCommand::QueryInventory).await?;
        let data = result.data.unwrap_or(serde_json::Value::Null);

        if let Some(arr) = data.as_array() {
            let inventory: Vec<Option<ItemStack>> = arr
                .iter()
                .map(|item| {
                    if item.is_null() {
                        None
                    } else {
                        let item_id = item.get("item_id")?.as_str()?.to_string();
                        let count_raw = item.get("count")?.as_u64()?;
                        let count = u8::try_from(count_raw).ok()?;
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
    /// 5. Equip the selected tool via the state machine's `EquipTool` step.
    /// 6. Walk to the block vicinity.
    /// 7. Verify arrival.
    /// 8. Start mining.
    /// 9. Wait for mining completion (sleep calculated from [`mining_calc`](crate::mining_calc)).
    /// 10. Verify the block is broken.
    /// 11. Return success or failure.
    pub(crate) async fn execute_mine_block<B: BotActions>(
        executor: &CommandExecutor<B>,
        pos: BlockPos,
        use_best_tool: bool,
    ) -> Result<BotResult, BotError> {
        trace!(?pos, use_best_tool, "execute_mine_block start");

        // Step 1: Check online
        if !executor.state.is_online() {
            warn!("bot offline, cannot mine block");
            return Err(BotError::Offline("bot is not connected".into()));
        }

        // Step 2: Query block type
        let snapshot = executor.state.read_snapshot();
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
        let mut skip_equip = false;

        if use_best_tool && required_tool != ToolType::Hand {
            let inventory = Self::query_inventory(executor).await?;
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

            // If the best tool is only in the main inventory (not hotbar),
            // we can't equip it — fall back to mining with whatever is held.
            // Skip the EquipTool state to avoid a guaranteed error.
            if selection.needs_move_to_hotbar {
                warn!(
                    ?tool_type,
                    "best tool is in main inventory, mining with current tool"
                );
                tool_type = ToolType::Hand;
                material = MaterialTier::Wood;
                skip_equip = true;
            }
        }

        // Build state machine
        let op = MineBlockOperation::new(pos, tool_type);
        let mut state = OperationState::Idle;

        // Step 6-10: Drive state machine
        state = op.advance(state, OperationEvent::Start);

        // If the best tool was only in the main inventory, skip equipping
        // and mine with whatever is currently held.
        if skip_equip {
            state = op.advance(state, OperationEvent::ToolEquipped);
        }

        while !matches!(state, OperationState::Completed | OperationState::Failed(_)) {
            match op.current_action(&state) {
                Some(BotCommand::MoveTo(target)) => {
                    trace!(?target, "dispatching MoveTo");
                    let result = executor.dispatch(BotCommand::MoveTo(target)).await?;
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
                    trace!(?t, "dispatching EquipTool");
                    let result = executor.dispatch(BotCommand::EquipTool(t)).await?;
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
                    trace!(?bp, "dispatching BreakBlock");
                    let result = executor.dispatch(BotCommand::BreakBlock(bp)).await?;
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
                    let new_snapshot = executor.state.read_snapshot();
                    // A post-mine snapshot may contain an "air" entry at this
                    // position; treat that as "block gone" rather than "still
                    // present" so successful mining isn't reported as failure.
                    let still_there = new_snapshot
                        .blocks
                        .iter()
                        .any(|b| b.position == pos && b.block_type != "air");
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
    #[allow(dead_code)] // part of the CompoundOpExecutor API; not yet wired into handle_act
    pub(crate) async fn execute_place_block<B: BotActions>(
        executor: &CommandExecutor<B>,
        pos: BlockPos,
        block_type: String,
    ) -> Result<BotResult, BotError> {
        trace!(?pos, %block_type, "execute_place_block start");

        if !executor.state.is_online() {
            return Err(BotError::Offline("bot is not connected".into()));
        }

        // Step 1: Find item in inventory
        let inventory = Self::query_inventory(executor).await?;
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
            .and_then(|i| u8::try_from(i).ok());

        if let Some(s) = slot {
            if s > 8 {
                return Err(BotError::Internal(format!(
                    "{block_type} is in main inventory slot {s}; move it to a hotbar slot (0-8) before placing"
                )));
            }
            executor.dispatch(BotCommand::SwitchHotbarSlot(s)).await?;
        }

        // Build state machine
        let op = PlaceBlockOperation::new(pos, block_type.clone(), ToolType::Hand);
        let mut state = OperationState::Idle;

        state = op.advance(state, OperationEvent::Start);
        // EquippingTool is skipped — item selection was handled above
        // via SwitchHotbarSlot, so advance past it with ToolEquipped.
        state = op.advance(state, OperationEvent::ToolEquipped);

        while !matches!(state, OperationState::Completed | OperationState::Failed(_)) {
            match op.current_action(&state) {
                Some(BotCommand::MoveTo(target)) => {
                    let result = executor.dispatch(BotCommand::MoveTo(target)).await?;
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
                    let result = executor
                        .dispatch(BotCommand::PlaceBlock(target, bt))
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
                    let new_snapshot = executor.state.read_snapshot();
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
    #[allow(dead_code)] // part of the CompoundOpExecutor API; not yet wired into handle_act
    pub(crate) async fn execute_open_container<B: BotActions>(
        executor: &CommandExecutor<B>,
        pos: BlockPos,
    ) -> Result<BotResult, BotError> {
        trace!(?pos, "execute_open_container start");

        if !executor.state.is_online() {
            return Err(BotError::Offline("bot is not connected".into()));
        }

        let op = OpenContainerOperation::new(pos);
        let mut state = OperationState::Idle;

        state = op.advance(state, OperationEvent::Start);

        while !matches!(state, OperationState::Completed | OperationState::Failed(_)) {
            match op.current_action(&state) {
                Some(BotCommand::MoveTo(target)) => {
                    let result = executor.dispatch(BotCommand::MoveTo(target)).await?;
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
                    let result = executor.dispatch(BotCommand::OpenContainer(target)).await?;
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
    #[allow(dead_code)] // part of the CompoundOpExecutor API; not yet wired into handle_act
    pub(crate) async fn execute_equip_tool<B: BotActions>(
        executor: &CommandExecutor<B>,
        tool_type: ToolType,
    ) -> Result<BotResult, BotError> {
        trace!(?tool_type, "execute_equip_tool start");

        if !executor.state.is_online() {
            return Err(BotError::Offline("bot is not connected".into()));
        }

        // Step 1: Find best tool
        let inventory = Self::query_inventory(executor).await?;
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

        // Tool equipping is handled entirely by the state machine's EquipTool
        // step; switching the hotbar slot manually here would both duplicate
        // that and fail for tools in the main inventory (slots 9-35). `slot`
        // is retained only for the result message below.

        // Step 3: Drive state machine
        let op = EquipToolOperation::new(tool_type);
        let mut state = OperationState::Idle;

        state = op.advance(state, OperationEvent::Start);

        while !matches!(state, OperationState::Completed | OperationState::Failed(_)) {
            match op.current_action(&state) {
                Some(BotCommand::EquipTool(t)) => {
                    let result = executor.dispatch(BotCommand::EquipTool(t)).await?;
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
    use crate::config::AppConfig;
    use crate::state::SharedState;
    use crate::types::{BlockEntry, GameMode, SelfPlayer, WorldSnapshot};
    use std::sync::Arc;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, Ordering};

    // ═══════════════════════════════════════════════════════════════
    // Mock bot — implements BotActions by mutating SharedState
    // ═══════════════════════════════════════════════════════════════

    /// Shared configurable state for the mock bot.
    ///
    /// All fields use interior mutability so `BotActions` methods (which take
    /// `&self`) can mutate behavior. The mock is held by value inside
    /// `CommandExecutor<MockBot>`, so all configuration must live behind
    /// `Arc` to be observable from the test after the executor is built.
    struct MockBotState {
        inventory: Mutex<Vec<Option<ItemStack>>>,
        /// If `true` (default), `goto` succeeds and updates player position.
        /// If `false`, `goto` returns `Err(PathfindingFailed)`.
        goto_succeeds: AtomicBool,
        /// If `true` (default), `mine_block` replaces the block with an "air"
        /// entry. If `false`, the block is left untouched — simulates a
        /// mining interruption where the block is still present after mining
        /// time elapses.
        mine_removes_block: AtomicBool,
        /// Block type that `block_interact` should "place" into the snapshot.
        /// Set by tests that exercise `execute_place_block`; `None` means
        /// `block_interact` is a no-op (no block is added).
        next_place_type: Mutex<Option<String>>,
    }

    impl MockBotState {
        fn new(inventory: Vec<Option<ItemStack>>) -> Self {
            Self {
                inventory: Mutex::new(inventory),
                goto_succeeds: AtomicBool::new(true),
                mine_removes_block: AtomicBool::new(true),
                next_place_type: Mutex::new(None),
            }
        }
    }

    /// Mock bot that implements [`BotActions`] by updating [`SharedState`].
    ///
    /// - `goto` updates `self_player.position` in the snapshot (configurable
    ///   to fail with `PathfindingFailed` via `goto_succeeds`).
    /// - `mine_block` replaces the block with an "air" entry (configurable
    ///   to leave the block via `mine_removes_block`, simulating a mining
    ///   interruption).
    /// - `block_interact` adds a pre-configured block type (`next_place_type`)
    ///   to the snapshot, mirroring a successful block placement.
    /// - `inventory_entries` returns the test inventory.
    /// - `switch_hotbar_slot` updates `held_item_slot` in the snapshot.
    /// - Other methods are no-ops or return defaults.
    struct MockBot {
        state: Arc<SharedState>,
        mock: Arc<MockBotState>,
    }

    impl BotActions for MockBot {
        async fn goto(&self, pos: &BlockPos) -> Result<(), BotError> {
            if !self.mock.goto_succeeds.load(Ordering::SeqCst) {
                return Err(BotError::PathfindingFailed {
                    target: *pos,
                    reason: "mock pathfinding failure".into(),
                });
            }
            let snap = (*self.state.read_snapshot()).clone();
            self.state.update_snapshot(WorldSnapshot {
                self_player: SelfPlayer {
                    position: *pos,
                    ..snap.self_player
                },
                ..snap
            });
            Ok(())
        }

        async fn jump(&self) {}

        fn teleport(&self, _pos: &BlockPos) {}

        fn switch_hotbar_slot(&self, slot: u8) {
            let snap = (*self.state.read_snapshot()).clone();
            self.state.update_snapshot(WorldSnapshot {
                self_player: SelfPlayer {
                    held_item_slot: slot,
                    ..snap.self_player
                },
                ..snap
            });
        }

        fn drop_item(&self, _slot: u8, _count: u8) {}

        fn start_use_item(&self) {}

        fn chat(&self, _message: &str) {}

        fn attack_entity(&self, _entity_id: u32) -> Result<(), BotError> {
            Ok(())
        }

        fn set_crouching(&self, _crouching: bool) {}

        fn mine_block(&self, pos: &BlockPos) {
            if !self.mock.mine_removes_block.load(Ordering::SeqCst) {
                // Simulate mining interruption: leave the block in place so
                // the post-mine verification detects "block still present".
                return;
            }
            // Replace the block with an "air" entry, mirroring a real
            // server's post-mine snapshot.
            let mut snap = (*self.state.read_snapshot()).clone();
            for b in snap.blocks.iter_mut() {
                if b.position == *pos {
                    b.block_type = "air".into();
                }
            }
            self.state.update_snapshot(snap);
        }

        fn block_interact(&self, pos: &BlockPos) {
            let bt = self.mock.next_place_type.lock().unwrap().clone();
            if let Some(bt) = bt {
                let mut snap = (*self.state.read_snapshot()).clone();
                let exists = snap.blocks.iter().any(|b| b.position == *pos);
                if exists {
                    for b in snap.blocks.iter_mut() {
                        if b.position == *pos {
                            b.block_type = bt.clone();
                        }
                    }
                } else {
                    snap.blocks.push(BlockEntry {
                        position: *pos,
                        block_type: bt,
                        block_state: None,
                    });
                }
                self.state.update_snapshot(snap);
            }
        }

        async fn open_container(&self, _pos: &BlockPos) -> Result<(), BotError> {
            Ok(())
        }

        fn inventory_entries(&self) -> Vec<Option<ItemStack>> {
            self.mock.inventory.lock().unwrap().clone()
        }
    }

    // ═══════════════════════════════════════════════════════════════
    // Snapshot helpers
    // ═══════════════════════════════════════════════════════════════

    fn make_snapshot_with_block(pos: BlockPos, block_type: &str) -> WorldSnapshot {
        let chunk_x = pos.x >> 4;
        let chunk_z = pos.z >> 4;
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
            // Include the chunk so `handle_break_block` considers it loaded.
            chunk_summary: vec![(chunk_x, chunk_z)],
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

    /// Set up a test executor backed by a [`MockBot`].
    ///
    /// Returns `(executor, mock_state, shared_state)`. Tests can configure
    /// mock behavior by mutating `mock_state` (e.g. set `goto_succeeds` to
    /// `false` to simulate pathfinding failure, or `mine_removes_block` to
    /// `false` to simulate a mining interruption).
    fn setup(
        inventory: Vec<Option<ItemStack>>,
        snapshot: WorldSnapshot,
    ) -> (
        CommandExecutor<MockBot>,
        Arc<MockBotState>,
        Arc<SharedState>,
    ) {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        state.update_snapshot(snapshot);
        state.set_online(true);
        let mock_state = Arc::new(MockBotState::new(inventory));
        let bot = MockBot {
            state: Arc::clone(&state),
            mock: Arc::clone(&mock_state),
        };
        let executor = CommandExecutor::new_for_lease(bot, Arc::clone(&state), None);
        (executor, mock_state, state)
    }

    /// Build an offline executor (no snapshot, online=false) for tests that
    /// only check the offline guard.
    fn offline_executor() -> CommandExecutor<MockBot> {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        state.set_online(false);
        let mock_state = Arc::new(MockBotState::new(vec![]));
        let bot = MockBot {
            state: Arc::clone(&state),
            mock: Arc::clone(&mock_state),
        };
        CommandExecutor::new_for_lease(bot, Arc::clone(&state), None)
    }

    // ═══════════════════════════════════════════════════════════════
    // execute_mine_block tests
    // ═══════════════════════════════════════════════════════════════

    // ── execute_mine_block: offline ───────────────────────────────────────

    #[tokio::test]
    async fn test_mine_block_offline() {
        let executor = offline_executor();

        let pos = BlockPos::new(10, 64, 20);
        let result = CompoundOpExecutor::execute_mine_block(&executor, pos, false).await;

        assert!(result.is_err());
        assert!(matches!(result, Err(BotError::Offline(_))));
    }

    // ── execute_mine_block: block not found ───────────────────────────────

    #[tokio::test]
    async fn test_mine_block_not_found() {
        let (executor, _mock, _state) = setup(vec![], make_empty_snapshot());

        let pos = BlockPos::new(10, 64, 20);
        let result = CompoundOpExecutor::execute_mine_block(&executor, pos, false).await;

        assert!(result.is_err());
        assert!(matches!(result, Err(BotError::BlockNotFound(_))));
    }

    // ── execute_mine_block: happy path (hand, no tool needed) ─────────────

    #[tokio::test]
    async fn test_mine_block_happy_path_hand() {
        let pos = BlockPos::new(10, 64, 20);
        let snapshot = make_snapshot_with_block(pos, "dirt");
        let (executor, _mock, _state) = setup(vec![], snapshot);

        let result = CompoundOpExecutor::execute_mine_block(&executor, pos, false).await;

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
        let (executor, _mock, state) = setup(inventory, snapshot);

        let result = CompoundOpExecutor::execute_mine_block(&executor, pos, true).await;

        assert!(result.is_ok(), "expected success, got: {:?}", result);
        let bot_result = result.unwrap();
        assert!(bot_result.success);
        assert!(bot_result.message.contains("Mined stone"));

        // Verify the block was replaced with "air" in the snapshot.
        let final_snapshot = state.read_snapshot();
        assert!(
            final_snapshot
                .blocks
                .iter()
                .any(|b| b.position == pos && b.block_type == "air"),
            "expected block at {:?} to be replaced with air, got blocks: {:?}",
            pos,
            final_snapshot.blocks
        );
    }

    // ── execute_mine_block: tool not found ────────────────────────────────

    #[tokio::test]
    async fn test_mine_block_tool_not_found() {
        let pos = BlockPos::new(10, 64, 20);
        let snapshot = make_snapshot_with_block(pos, "stone");
        let (executor, _mock, _state) = setup(vec![], snapshot);

        let result = CompoundOpExecutor::execute_mine_block(&executor, pos, true).await;

        assert!(result.is_err());
        assert!(matches!(result, Err(BotError::ToolNotFound { .. })));
    }

    // ── execute_mine_block: mining interrupted (block remains) ───────────

    #[tokio::test]
    async fn test_mine_block_mining_interrupted() {
        // The mock leaves the block in place after mining (simulating an
        // interruption). The post-mine verification detects "block still
        // present" and fails with MiningInterrupted.
        let pos = BlockPos::new(10, 64, 20);
        let snapshot = make_snapshot_with_block(pos, "stone");
        let inventory = vec![
            None,
            Some(ItemStack {
                item_id: "iron_pickaxe".into(),
                count: 1,
            }),
        ];
        let (executor, mock, _state) = setup(inventory, snapshot);
        mock.mine_removes_block.store(false, Ordering::SeqCst);

        let result = CompoundOpExecutor::execute_mine_block(&executor, pos, true).await;

        assert!(result.is_err());
        assert!(matches!(result, Err(BotError::MiningInterrupted { .. })));
    }

    // ── execute_mine_block: air entry after mining counts as broken ────────

    #[tokio::test]
    async fn test_mine_block_air_entry_counts_as_broken() {
        // Real servers replace a mined block with an "air" entry rather than
        // removing it from the snapshot. The verification must treat "air" as
        // "block gone" — otherwise every successful mine is reported as failure.
        let pos = BlockPos::new(10, 64, 20);
        let snapshot = make_snapshot_with_block(pos, "stone");
        let inventory = vec![
            None,
            Some(ItemStack {
                item_id: "iron_pickaxe".into(),
                count: 1,
            }),
        ];
        let (executor, _mock, _state) = setup(inventory, snapshot);

        let result = CompoundOpExecutor::execute_mine_block(&executor, pos, true).await;

        assert!(result.is_ok(), "expected success, got: {:?}", result);
        let bot_result = result.unwrap();
        assert!(bot_result.success);
        assert!(bot_result.message.contains("Mined stone"));
    }

    // ── execute_mine_block: pathfinding fails ─────────────────────────────

    #[tokio::test]
    async fn test_mine_block_pathfinding_fails() {
        let pos = BlockPos::new(10, 64, 20);
        let snapshot = make_snapshot_with_block(pos, "stone");
        let (executor, mock, _state) = setup(vec![], snapshot);
        mock.goto_succeeds.store(false, Ordering::SeqCst);

        let result = CompoundOpExecutor::execute_mine_block(&executor, pos, false).await;

        assert!(result.is_err());
        assert!(matches!(result, Err(BotError::PathfindingFailed { .. })));
    }

    // ═══════════════════════════════════════════════════════════════
    // execute_place_block tests
    // ═══════════════════════════════════════════════════════════════

    // ── execute_place_block: offline ──────────────────────────────────────

    #[tokio::test]
    async fn test_place_block_offline() {
        let executor = offline_executor();

        let pos = BlockPos::new(10, 64, 20);
        let result = CompoundOpExecutor::execute_place_block(&executor, pos, "stone".into()).await;

        assert!(result.is_err());
        assert!(matches!(result, Err(BotError::Offline(_))));
    }

    // ── execute_place_block: no item ─────────────────────────────────────

    #[tokio::test]
    async fn test_place_block_no_item() {
        let pos = BlockPos::new(10, 64, 20);
        let snapshot = make_empty_snapshot();
        let (executor, _mock, _state) = setup(vec![], snapshot);

        let result = CompoundOpExecutor::execute_place_block(&executor, pos, "stone".into()).await;

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
        let (executor, mock, state) = setup(inventory, snapshot);
        // Pre-configure the mock so `block_interact` places a "stone" block.
        *mock.next_place_type.lock().unwrap() = Some("stone".into());

        let result = CompoundOpExecutor::execute_place_block(&executor, pos, "stone".into()).await;

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

    // ═══════════════════════════════════════════════════════════════
    // execute_open_container tests
    // ═══════════════════════════════════════════════════════════════

    // ── execute_open_container: offline ───────────────────────────────────

    #[tokio::test]
    async fn test_open_container_offline() {
        let executor = offline_executor();

        let pos = BlockPos::new(10, 64, 20);
        let result = CompoundOpExecutor::execute_open_container(&executor, pos).await;

        assert!(result.is_err());
        assert!(matches!(result, Err(BotError::Offline(_))));
    }

    // ── execute_open_container: happy path ──────────────────────────────

    #[tokio::test]
    async fn test_open_container_happy_path() {
        let pos = BlockPos::new(10, 64, 20);
        let snapshot = make_empty_snapshot();
        let (executor, _mock, _state) = setup(vec![], snapshot);

        let result = CompoundOpExecutor::execute_open_container(&executor, pos).await;

        assert!(result.is_ok(), "expected success, got: {:?}", result);
        let bot_result = result.unwrap();
        assert!(bot_result.success);
        assert!(bot_result.message.contains("Opened container"));
    }

    // ═══════════════════════════════════════════════════════════════
    // execute_equip_tool tests
    // ═══════════════════════════════════════════════════════════════

    // ── execute_equip_tool: offline ─────────────────────────────────────

    #[tokio::test]
    async fn test_equip_tool_offline() {
        let executor = offline_executor();

        let result = CompoundOpExecutor::execute_equip_tool(&executor, ToolType::Pickaxe).await;

        assert!(result.is_err());
        assert!(matches!(result, Err(BotError::Offline(_))));
    }

    // ── execute_equip_tool: not found ───────────────────────────────────

    #[tokio::test]
    async fn test_equip_tool_not_found() {
        let snapshot = make_empty_snapshot();
        let (executor, _mock, _state) = setup(vec![], snapshot);

        let result = CompoundOpExecutor::execute_equip_tool(&executor, ToolType::Pickaxe).await;

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
        let (executor, _mock, state) = setup(inventory, snapshot);

        let result = CompoundOpExecutor::execute_equip_tool(&executor, ToolType::Pickaxe).await;

        assert!(result.is_ok(), "expected success, got: {:?}", result);
        let bot_result = result.unwrap();
        assert!(bot_result.success);
        assert!(bot_result.message.contains("Equipped Pickaxe"));

        // Verify held slot was updated (diamond_pickaxe is at slot 0)
        let final_snapshot = state.read_snapshot();
        assert_eq!(final_snapshot.self_player.held_item_slot, 0);
    }

    // ── execute_equip_tool: selects best tier ─────────────────────────────

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
        let (executor, _mock, state) = setup(inventory, snapshot);

        let result = CompoundOpExecutor::execute_equip_tool(&executor, ToolType::Pickaxe).await;

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
        let (executor, _mock, state) = setup(vec![], snapshot);

        let result = CompoundOpExecutor::execute_equip_tool(&executor, ToolType::Hand).await;

        assert!(result.is_ok(), "expected success, got: {:?}", result);
        let bot_result = result.unwrap();
        assert!(bot_result.success);
        assert!(bot_result.message.contains("Equipped Hand"));

        // Verify held slot was NOT changed (no SwitchHotbarSlot sent)
        let final_snapshot = state.read_snapshot();
        assert_eq!(final_snapshot.self_player.held_item_slot, 3);
    }

    // ═══════════════════════════════════════════════════════════════
    // State machine integration tests
    // ═══════════════════════════════════════════════════════════════

    // ── State machine integration: mine block reaches all states ─────────

    #[tokio::test]
    async fn test_mine_block_state_machine_reaches_all_states() {
        let pos = BlockPos::new(5, 64, 5);
        let snapshot = make_snapshot_with_block(pos, "dirt");
        let (executor, _mock, _state) = setup(vec![], snapshot);

        let result = CompoundOpExecutor::execute_mine_block(&executor, pos, false).await;
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
        let (executor, mock, _state) = setup(inventory, snapshot);
        *mock.next_place_type.lock().unwrap() = Some("oak_planks".into());

        let result =
            CompoundOpExecutor::execute_place_block(&executor, pos, "oak_planks".into()).await;
        assert!(result.is_ok());
    }

    // ── State machine integration: open container reaches all states ──────

    #[tokio::test]
    async fn test_open_container_state_machine_reaches_all_states() {
        let pos = BlockPos::new(5, 64, 5);
        let snapshot = make_empty_snapshot();
        let (executor, _mock, _state) = setup(vec![], snapshot);

        let result = CompoundOpExecutor::execute_open_container(&executor, pos).await;
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
        let (executor, _mock, _state) = setup(inventory, snapshot);

        let result = CompoundOpExecutor::execute_equip_tool(&executor, ToolType::Axe).await;
        assert!(result.is_ok());
    }
}
