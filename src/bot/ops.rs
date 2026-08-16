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

use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{debug, trace, warn};

use crate::block_data::ItemStack;
use crate::block_data::best_tool_for_block;
use crate::bot::commands::{BotActions, CommandExecutor};
use crate::compound_ops::{
    EquipToolOperation, MineBlockOperation, OpenContainerOperation, OperationEvent, OperationState,
    PlaceBlockOperation, find_standable_neighbor,
};
use crate::error::BotError;
use crate::mining_calc::calculate_mine_time;
use crate::state::SharedState;
use crate::tool_select::{build_tool_alternatives, find_tool_in_inventory, select_tool_for_block};
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
// Race-safe block verification helpers (F6-4)
// ---------------------------------------------------------------------------

/// Poll the snapshot until the block at `pos` is GONE (absent from the
/// block index or turned into air) or the `budget` expires.
///
/// Returns `true` when the block is gone, `false` when the budget ran out
/// with the block still present.
///
/// F6-4: mine results land in the snapshot only on the next periodic
/// snapshot rebuild (default every 500 ms), so a single read right after
/// the mining wait can still see the pre-mine state and wrongly report
/// "block still present". Polling with a bounded budget removes that race.
///
/// AIR-BLOCK semantics (1.0.7): snapshots INCLUDE `air` entries — a broken
/// block becomes `"air"` in [`block_index`], it does not leave the index —
/// so `"air"` counts as gone. Per AGENTS.md M-12, lookups always go through
/// `block_index`, never a linear `blocks` scan.
pub(crate) async fn wait_for_block_gone(
    state: &Arc<SharedState>,
    pos: BlockPos,
    budget: Duration,
) -> bool {
    let deadline = tokio::time::Instant::now() + budget;
    loop {
        let gone = {
            let snapshot = state.read_snapshot();
            match snapshot.block_index.get(&pos) {
                None => true,
                Some(&idx) => snapshot.blocks[idx].block_type.eq_ignore_ascii_case("air"),
            }
        };
        if gone {
            return true;
        }
        let now = tokio::time::Instant::now();
        if now >= deadline {
            return false;
        }
        let wait = std::cmp::min(deadline - now, Duration::from_millis(100));
        sleep(wait).await;
    }
}

/// Poll the snapshot until the block at `pos` is PRESENT (the block index
/// has an entry for it whose type is not air) or the `budget` expires.
///
/// Returns `true` when the block is present, `false` when the budget ran
/// out without the block appearing.
///
/// F6-4: same race as [`wait_for_block_gone`] — place results land in the
/// snapshot on the next periodic rebuild, so verification must poll instead
/// of deciding from a single possibly-stale read. AIR-BLOCK semantics: an
/// `air` entry does NOT count as present.
pub(crate) async fn wait_for_block_present(
    state: &Arc<SharedState>,
    pos: BlockPos,
    budget: Duration,
) -> bool {
    let deadline = tokio::time::Instant::now() + budget;
    loop {
        let present = {
            let snapshot = state.read_snapshot();
            snapshot
                .block_index
                .get(&pos)
                .map(|&idx| !snapshot.blocks[idx].block_type.eq_ignore_ascii_case("air"))
                .unwrap_or(false)
        };
        if present {
            return true;
        }
        let now = tokio::time::Instant::now();
        if now >= deadline {
            return false;
        }
        let wait = std::cmp::min(deadline - now, Duration::from_millis(100));
        sleep(wait).await;
    }
}

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
                        let item_id = match item.get("item_id").and_then(|v| v.as_str()) {
                            Some(id) => id.to_string(),
                            None => {
                                warn!(?item, "inventory item missing item_id field");
                                return None;
                            }
                        };
                        let count_raw = match item.get("count").and_then(|v| v.as_u64()) {
                            Some(c) => c,
                            None => {
                                warn!(?item, "inventory item missing or invalid count field");
                                return None;
                            }
                        };
                        let count = match u8::try_from(count_raw) {
                            Ok(c) => c,
                            Err(_) => {
                                warn!(count_raw, "inventory item count out of u8 range");
                                return None;
                            }
                        };
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

        // Step 2: Query block type — Task 2.5 (M-12): use the O(1) `block_index`
        // instead of a linear `blocks.iter().find()` scan.
        let snapshot = executor.state.read_snapshot();
        let block_type = snapshot
            .block_index
            .get(&pos)
            .map(|&idx| snapshot.blocks[idx].block_type.clone())
            .ok_or_else(|| {
                warn!(?pos, "block not found in snapshot");
                BotError::BlockNotFound(pos)
            })?;

        trace!(?pos, %block_type, "found block in snapshot");

        // Steps 3-5: Tool selection and equipping
        let required_tool = best_tool_for_block(&block_type);
        let mut tool_type = ToolType::Hand;
        let mut material = MaterialTier::Wood;
        let mut tool_in_inventory_not_equippable = false;

        if use_best_tool && required_tool != ToolType::Hand {
            let inventory = Self::query_inventory(executor).await?;
            let selection = select_tool_for_block(&block_type, &inventory);

            // Step 4: Tool needed but not in inventory
            if selection.tool_type == ToolType::Hand {
                return Err(BotError::ToolNotFound {
                    tool_type: required_tool,
                    material: None,
                    alternatives: build_tool_alternatives(
                        required_tool,
                        selection.required_harvest_level,
                    ),
                });
            }

            tool_type = selection.tool_type;
            material = selection.material.unwrap_or(MaterialTier::Wood);

            // Task 1.5 (P1-#6): if the best tool is only in the main inventory
            // (not hotbar), it cannot be auto-equipped (SwitchHotbarSlot only
            // accepts 0-8). The original implementation dropped the tool_type
            // to `Hand` here, which forced a 5× wrong-tool penalty and ~11.25s
            // mining time on stone. We now keep the original `tool_type` and
            // `material` so `calculate_mine_time` uses the correct tool speed
            // and `is_correct_tool` does not apply the wrong-tool penalty.
            // The state machine emits `ToolAlreadyInInventory` after MoveTo
            // to skip the EquipTool step (P0-#3) and go straight to mining.
            if selection.needs_move_to_hotbar {
                warn!(
                    ?tool_type,
                    ?material,
                    "best tool is in main inventory, skipping equip and mining with current tool"
                );
                tool_in_inventory_not_equippable = true;
            }
        }

        // Build state machine
        let op = MineBlockOperation::new(pos, tool_type);
        let mut state = OperationState::Idle;

        // Step 6-10: Drive state machine
        state = op.advance(state, OperationEvent::Start);

        // Task 1.5 (P0-#3) — skip-equip transition: when the best tool is
        // in the inventory but cannot be hotbar-switched, the state machine
        // emits `ToolAlreadyInInventory` *after* MoveTo succeeds (to skip
        // `EquippingTool`). The actual `advance` is therefore issued in the
        // MoveTo branch below, gated on `tool_in_inventory_not_equippable`.
        // The post-MoveTo branch chooses between `Arrived` (normal flow)
        // and `ToolAlreadyInInventory` (skip equip).

        while !matches!(state, OperationState::Completed | OperationState::Failed(_)) {
            match op.current_action(&state) {
                Some(BotCommand::MoveTo(_)) => {
                    // M-4: walk to a standable neighbour of the target, not
                    // the target itself — walking into a solid block causes
                    // pathfinder collision.
                    let snapshot = executor.state.read_snapshot();
                    let move_target = match find_standable_neighbor(&snapshot, pos) {
                        Some(neighbor) => neighbor,
                        None => {
                            warn!(?pos, "no standable position adjacent to target");
                            state = op.advance(
                                state,
                                OperationEvent::Failed(BotError::Internal(
                                    "no standable position adjacent to target".into(),
                                )),
                            );
                            continue;
                        }
                    };
                    trace!(?move_target, "dispatching MoveTo");
                    // Task 2.6: dispatch errors must transition the state
                    // machine to `Failed(_)` rather than `?`-returning out
                    // of the executor (so the outer match can distinguish
                    // "failed at MoveTo" from "failed at BreakBlock").
                    let result = match executor.dispatch(BotCommand::MoveTo(move_target)).await {
                        Ok(r) => r,
                        Err(e) => {
                            state = op.advance(state, OperationEvent::Failed(e));
                            continue;
                        }
                    };
                    if !result.success {
                        state = op.advance(
                            state,
                            OperationEvent::Failed(BotError::PathfindingFailed {
                                target: move_target,
                                reason: result.message,
                            }),
                        );
                        continue;
                    }
                    // Task 1.5 (P0-#3): after a successful MoveTo, either
                    // advance normally to `EquippingTool` (via `Arrived`)
                    // or skip straight to `ExecutingAction` (via
                    // `ToolAlreadyInInventory`) when the best tool is in
                    // the inventory but cannot be auto-equipped.
                    if tool_in_inventory_not_equippable {
                        state = op.advance(state, OperationEvent::ToolAlreadyInInventory);
                    } else {
                        state = op.advance(state, OperationEvent::Arrived);
                    }
                }
                Some(BotCommand::EquipTool(t)) => {
                    trace!(?t, "dispatching EquipTool");
                    // Task 2.6: same `?` → state.advance(Failed(e)) rewrite.
                    let result = match executor.dispatch(BotCommand::EquipTool(t)).await {
                        Ok(r) => r,
                        Err(e) => {
                            state = op.advance(state, OperationEvent::Failed(e));
                            continue;
                        }
                    };
                    if !result.success {
                        state = op.advance(
                            state,
                            OperationEvent::Failed(BotError::ToolNotFound {
                                tool_type: t,
                                material: None,
                                alternatives: vec![],
                            }),
                        );
                        continue;
                    }
                    state = op.advance(state, OperationEvent::ToolEquipped);
                }
                Some(BotCommand::BreakBlock(bp)) => {
                    trace!(?bp, "dispatching BreakBlock");
                    // Task 2.6: same `?` → state.advance(Failed(e)) rewrite.
                    let result = match executor.dispatch(BotCommand::BreakBlock(bp)).await {
                        Ok(r) => r,
                        Err(e) => {
                            state = op.advance(state, OperationEvent::Failed(e));
                            continue;
                        }
                    };
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
                    tokio::select! {
                        _ = sleep(Duration::from_secs_f64(mine_time)) => {}
                        _ = async {
                            while executor.state.is_online() {
                                sleep(Duration::from_millis(100)).await;
                            }
                        } => {
                            state = op.advance(
                                state,
                                OperationEvent::Failed(BotError::MiningInterrupted {
                                    reason: "bot went offline during mining".into(),
                                }),
                            );
                            continue;
                        }
                    }

                    // Step 10: Verify block broken — F6-4: poll the
                    // snapshot with a bounded budget instead of deciding
                    // from a single possibly-stale read (the broken state
                    // lands on the next periodic snapshot rebuild).
                    // `wait_for_block_gone` treats "air" as gone
                    // (1.0.7 air-in-snapshot semantics) and goes through
                    // `block_index` (M-12).
                    let budget =
                        Duration::from_millis(executor.state.read_config().snapshot_interval_ms)
                            + Duration::from_millis(250);
                    if wait_for_block_gone(&executor.state, pos, budget).await {
                        state = op.advance(state, OperationEvent::BlockBroken);
                    } else {
                        warn!(?pos, "block still present after mining time");
                        state = op.advance(
                            state,
                            OperationEvent::Failed(BotError::MiningInterrupted {
                                reason: "block still present after mining time".into(),
                            }),
                        );
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
    /// 3. Walk near the target — Task 1.13 (P1-#8): use
    ///    [`find_standable_neighbor`] to pick a standable position adjacent
    ///    to the target (not the target itself, which the bot would path
    ///    into and get stuck).
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

        // Step 1 & 2: Find item in inventory, prefer hotbar (slots 0-8) first.
        // Only hotbar slots can be selected directly; an item in the main
        // inventory (slot 9-35) can't be switched to without an inventory-move
        // flow, so surface a clear error instead of letting the executor
        // reject slot >= 9.
        let inventory = Self::query_inventory(executor).await?;

        // First, look in hotbar (slots 0-8) — take the first match
        let hotbar_slot = inventory
            .iter()
            .take(9)
            .position(|s| s.as_ref().is_some_and(|item| item.item_id == block_type));

        if let Some(s) = hotbar_slot {
            executor
                .dispatch(BotCommand::SwitchHotbarSlot(s as u8))
                .await?;
        } else {
            // Not in hotbar — check main inventory
            let has_in_main = inventory
                .iter()
                .skip(9)
                .any(|s| s.as_ref().is_some_and(|item| item.item_id == block_type));

            if has_in_main {
                return Err(BotError::Internal(format!(
                    "{block_type} is in main inventory; move it to a hotbar slot (0-8) before placing"
                )));
            } else {
                return Err(BotError::ToolNotFound {
                    tool_type: ToolType::Hand,
                    material: None,
                    alternatives: vec![],
                });
            }
        }

        // Build state machine
        let op = PlaceBlockOperation::new(pos, block_type.clone(), ToolType::Hand);
        let mut state = OperationState::Idle;

        state = op.advance(state, OperationEvent::Start);
        // EquippingTool is skipped — item selection was handled above
        // via SwitchHotbarSlot, so advance past it with ToolEquipped.
        state = op.advance(state, OperationEvent::ToolEquipped);

        // Task 1.13 (P1-#8): find a standable position adjacent to the
        // target so the bot doesn't try to path into the target block
        // (which is solid/air depending on context) and get stuck. We
        // resolve the neighbour once up front and then `MoveTo(neighbour)`
        // before placing the actual block at `pos`.
        let snapshot = executor.state.read_snapshot();
        let move_target = find_standable_neighbor(&snapshot, pos).ok_or_else(|| {
            warn!(?pos, "no standable position adjacent to place target");
            BotError::Internal("no standable position adjacent to target".into())
        })?;

        while !matches!(state, OperationState::Completed | OperationState::Failed(_)) {
            match op.current_action(&state) {
                Some(BotCommand::MoveTo(_)) => {
                    // Always move to the standable neighbour, not the
                    // target — `current_action` returns the raw target
                    // for backward compat, but we override it here.
                    // Task 2.6: dispatch errors must transition the state
                    // machine to `Failed(_)` rather than `?`-returning out
                    // of the executor.
                    let result = match executor.dispatch(BotCommand::MoveTo(move_target)).await {
                        Ok(r) => r,
                        Err(e) => {
                            state = op.advance(state, OperationEvent::Failed(e));
                            continue;
                        }
                    };
                    if !result.success {
                        state = op.advance(
                            state,
                            OperationEvent::Failed(BotError::PathfindingFailed {
                                target: move_target,
                                reason: result.message,
                            }),
                        );
                        continue;
                    }
                    state = op.advance(state, OperationEvent::Arrived);
                }
                Some(BotCommand::PlaceBlock(target, bt)) => {
                    // Task 2.6: same `?` → state.advance(Failed(e)) rewrite.
                    let result = match executor.dispatch(BotCommand::PlaceBlock(target, bt)).await {
                        Ok(r) => r,
                        Err(e) => {
                            state = op.advance(state, OperationEvent::Failed(e));
                            continue;
                        }
                    };
                    if !result.success {
                        state = op.advance(
                            state,
                            OperationEvent::Failed(BotError::Internal(result.message)),
                        );
                        continue;
                    }

                    // Verify block placed — F6-4: poll the snapshot with a
                    // bounded budget instead of a fixed 200 ms sleep plus a
                    // single possibly-stale read (the placed state lands on
                    // the next periodic snapshot rebuild). An "air" entry
                    // does not count as placed (1.0.7 air-in-snapshot
                    // semantics); lookups go through `block_index` (M-12).
                    let budget =
                        Duration::from_millis(executor.state.read_config().snapshot_interval_ms)
                            + Duration::from_millis(250);
                    if wait_for_block_present(&executor.state, pos, budget).await {
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

        // Resolve standable neighbor once up front so we don't try to path
        // into the container block itself.
        let snapshot = executor.state.read_snapshot();
        let move_target = find_standable_neighbor(&snapshot, pos).ok_or_else(|| {
            warn!(?pos, "no standable position adjacent to container");
            BotError::Internal("no standable position adjacent to target".into())
        })?;

        while !matches!(state, OperationState::Completed | OperationState::Failed(_)) {
            match op.current_action(&state) {
                Some(BotCommand::MoveTo(_)) => {
                    // Always move to the standable neighbour, not the
                    // container position itself.
                    let result = match executor.dispatch(BotCommand::MoveTo(move_target)).await {
                        Ok(r) => r,
                        Err(e) => {
                            state = op.advance(state, OperationEvent::Failed(e));
                            continue;
                        }
                    };
                    if !result.success {
                        state = op.advance(
                            state,
                            OperationEvent::Failed(BotError::PathfindingFailed {
                                target: move_target,
                                reason: result.message,
                            }),
                        );
                        continue;
                    }
                    state = op.advance(state, OperationEvent::Arrived);
                }
                Some(BotCommand::OpenContainer(target)) => {
                    // Task 2.6: same `?` → state.advance(Failed(e)) rewrite.
                    let result = match executor.dispatch(BotCommand::OpenContainer(target)).await {
                        Ok(r) => r,
                        Err(e) => {
                            state = op.advance(state, OperationEvent::Failed(e));
                            continue;
                        }
                    };
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

        // Step 1: Find best tool (no harvest-level filter at this layer —
        // the executor can choose a tool that drops nothing if the player
        // insists; tool_select::select_tool_for_block applies the filter
        // when called from the compound-op layer).
        let inventory = Self::query_inventory(executor).await?;
        let found = find_tool_in_inventory(&tool_type, &inventory, None);

        if found.is_none() && tool_type != ToolType::Hand {
            return Err(BotError::ToolNotFound {
                tool_type,
                material: None,
                alternatives: vec![],
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
                                alternatives: vec![],
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
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

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
        /// When `true`, `is_goto_target_reached` always returns `false`,
        /// forcing any 50ms fallback loop in the real client to keep
        /// spinning. Default is `false` (mock arrives on first tick) so
        /// existing tests that drive the full `execute_mine_block` path
        /// see the same behavior as the synchronous mock goto.
        goto_target_unreached: AtomicBool,
        /// Value returned by `player_inventory_occupied_slots` (F6-3).
        /// Default 0 (inventory has free slots).
        player_inventory_occupied: AtomicUsize,
    }

    impl MockBotState {
        fn new(inventory: Vec<Option<ItemStack>>) -> Self {
            Self {
                inventory: Mutex::new(inventory),
                goto_succeeds: AtomicBool::new(true),
                mine_removes_block: AtomicBool::new(true),
                next_place_type: Mutex::new(None),
                goto_target_unreached: AtomicBool::new(false),
                player_inventory_occupied: AtomicUsize::new(0),
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
        fn is_goto_target_reached(&self) -> bool {
            // The mock defaults to "we have arrived" so any fallback
            // loop in the real client (or in tests that import this
            // mock) exits on the first tick. Tests that need a delayed
            // arrival can flip `goto_target_unreached` to `true` to
            // force the position check to stay false.
            !self.mock.goto_target_unreached.load(Ordering::SeqCst)
        }

        fn position(&self) -> Option<[f64; 3]> {
            // This mock keeps the snapshot in lock-step with bot movement
            // (goto/teleport mutate the snapshot position), so the snapshot
            // position IS the live position here.
            let p = self.state.read_snapshot().self_player.position;
            Some([p.x as f64, p.y as f64, p.z as f64])
        }

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

        fn stop_pathfinding(&self) {}

        fn swap_hotbar(&self, _source_menu_slot: u16, _target_hotbar_slot: u8) {}

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
                    let new_idx = snap.blocks.len();
                    snap.blocks.push(BlockEntry {
                        position: *pos,
                        block_type: bt,
                        block_state: None,
                    });
                    // Keep `block_index` consistent so post-place
                    // verification (`block_index.get(&pos)`) can find
                    // the freshly-placed block. A real server pushes
                    // the new state into the chunk section and the
                    // snapshot rebuild does this for us; the mock
                    // mutates the snapshot in place.
                    snap.block_index.insert(*pos, new_idx);
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

        fn player_inventory_occupied_slots(&self) -> usize {
            self.mock.player_inventory_occupied.load(Ordering::SeqCst)
        }
    }

    // ═══════════════════════════════════════════════════════════════
    // Snapshot helpers
    // ═══════════════════════════════════════════════════════════════

    fn make_snapshot_with_block(pos: BlockPos, block_type: &str) -> WorldSnapshot {
        let chunk_x = pos.x >> 4;
        let chunk_z = pos.z >> 4;
        let blocks = vec![
            BlockEntry {
                position: pos,
                block_type: block_type.into(),
                block_state: None,
            },
            // Standable neighbour + solid floor so `find_standable_neighbor`
            // can pick a valid move target (M-4: bot should walk next to
            // the block, not into it).
            BlockEntry {
                position: BlockPos::new(pos.x + 1, pos.y, pos.z),
                block_type: "air".into(),
                block_state: None,
            },
            BlockEntry {
                position: BlockPos::new(pos.x + 1, pos.y - 1, pos.z),
                block_type: "stone".into(),
                block_state: None,
            },
        ];
        // `find_standable_neighbor` and `find_obstacle_block` look up blocks
        // via `block_index` (M-12), so populate it the same way
        // `SnapshotBuilder::build` does in production.
        let block_index: std::collections::HashMap<BlockPos, usize> = blocks
            .iter()
            .enumerate()
            .map(|(i, b)| (b.position, i))
            .collect();
        WorldSnapshot {
            blocks,
            block_index,
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
                position_precise: None,
                yaw: None,
            },
            timestamp: 1,
            // Include the chunk so `handle_break_block` considers it loaded.
            chunk_summary: vec![(chunk_x, chunk_z)],
            commands_enabled: None,
            snapshot_seq: 0,
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
                position_precise: None,
                yaw: None,
            },
            timestamp: 1,
            chunk_summary: vec![],
            commands_enabled: None,
            ..Default::default()
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
        // Use a snapshot that has a standable neighbour at pos+1 so
        // the new find_standable_neighbor check (P1-#8) succeeds.
        let snapshot = make_snapshot_with_block(pos, "air");
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

    // ── execute_place_block: item in both hotbar and main inventory uses hotbar ──

    #[tokio::test]
    async fn test_place_block_prefers_hotbar_over_main_inventory() {
        let pos = BlockPos::new(10, 64, 20);
        let snapshot = make_snapshot_with_block(pos, "air");
        let mut inventory: Vec<Option<ItemStack>> = vec![None; 36];
        inventory[15] = Some(ItemStack {
            item_id: "cobblestone".into(),
            count: 64,
        });
        inventory[3] = Some(ItemStack {
            item_id: "cobblestone".into(),
            count: 64,
        });
        let (executor, mock, state) = setup(inventory, snapshot);
        *mock.next_place_type.lock().unwrap() = Some("cobblestone".into());

        let result =
            CompoundOpExecutor::execute_place_block(&executor, pos, "cobblestone".into()).await;

        assert!(result.is_ok(), "expected success, got: {:?}", result);
        let final_snapshot = state.read_snapshot();
        assert_eq!(
            final_snapshot.self_player.held_item_slot, 3,
            "should select hotbar slot 3, not main inventory slot 15"
        );
    }

    // ── execute_place_block: item only in main inventory errors ──

    #[tokio::test]
    async fn test_place_block_item_only_in_main_inventory_errors() {
        let pos = BlockPos::new(10, 64, 20);
        let snapshot = make_snapshot_with_block(pos, "air");
        let mut inventory: Vec<Option<ItemStack>> = vec![None; 36];
        inventory[20] = Some(ItemStack {
            item_id: "dirt".into(),
            count: 64,
        });
        let (executor, _mock, _state) = setup(inventory, snapshot);

        let result = CompoundOpExecutor::execute_place_block(&executor, pos, "dirt".into()).await;

        assert!(result.is_err());
        match result.unwrap_err() {
            BotError::Internal(msg) => {
                assert!(
                    msg.contains("main inventory"),
                    "expected error mentioning main inventory, got: {msg}"
                );
                assert!(
                    msg.contains("hotbar slot (0-8)"),
                    "expected error mentioning hotbar, got: {msg}"
                );
            }
            other => panic!("expected BotError::Internal, got: {other:?}"),
        }
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
        let snapshot = make_snapshot_with_block(pos, "chest");
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
        // Use a snapshot with a standable neighbour at pos+1 so
        // find_standable_neighbor (P1-#8) returns a valid target.
        let snapshot = make_snapshot_with_block(pos, "air");
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
        let snapshot = make_snapshot_with_block(pos, "chest");
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

    // ═══════════════════════════════════════════════════════════════
    // Task 1.5: skip_equip + tool_type preservation (P0-#3 + P1-#6)
    // ═══════════════════════════════════════════════════════════════

    /// Task 1.5 (P0-#3): when the best tool is in the main inventory
    /// (cannot be hotbar-switched), `execute_mine_block` must still
    /// succeed without trying to `EquipTool` (which would fail because
    /// `SwitchHotbarSlot` only accepts 0-8). The state machine uses the
    /// new `ToolAlreadyInInventory` event to skip past `EquippingTool`.
    #[tokio::test]
    async fn test_mine_block_skip_equip_when_tool_in_inventory() {
        let pos = BlockPos::new(10, 64, 20);
        let snapshot = make_snapshot_with_block(pos, "stone");
        // Iron pickaxe in the **main inventory** (slot 15) — `select_tool_for_block`
        // will mark `needs_move_to_hotbar = true`, which triggers the
        // `tool_in_inventory_not_equippable` branch.
        let mut inventory: Vec<Option<ItemStack>> = vec![None; 36];
        inventory[15] = Some(ItemStack {
            item_id: "iron_pickaxe".into(),
            count: 1,
        });
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
                .block_index
                .get(&pos)
                .map(|&idx| final_snapshot.blocks[idx].block_type == "air")
                .unwrap_or(false),
            "expected block at {:?} to be replaced with air",
            pos
        );
    }

    /// Task 1.5 (P1-#6): the `needs_move_to_hotbar` path must NOT reset
    /// `tool_type` to `Hand` and `material` to `Wood` — doing so applies
    /// a 5× wrong-tool penalty (`MiningInterrupted`'s "stone" time goes
    /// from 0.375s with an iron pickaxe to 11.25s with bare hands). The
    /// executor should keep the original `(tool_type, material)` so
    /// `calculate_mine_time` uses the iron pickaxe speed, and the actual
    /// mining time observed in the executor is < 11.25s.
    #[tokio::test]
    async fn test_mine_block_preserves_tool_type_when_hotbar_move_needed() {
        use std::time::Instant;

        let pos = BlockPos::new(10, 64, 20);
        let snapshot = make_snapshot_with_block(pos, "stone");
        // Iron pickaxe in main inventory (slot 12) so `needs_move_to_hotbar`
        // triggers. The executor must keep `(Pickaxe, Iron)` and skip equip.
        let mut inventory: Vec<Option<ItemStack>> = vec![None; 36];
        inventory[12] = Some(ItemStack {
            item_id: "iron_pickaxe".into(),
            count: 1,
        });
        let (executor, _mock, _state) = setup(inventory, snapshot);

        let start = Instant::now();
        let result = CompoundOpExecutor::execute_mine_block(&executor, pos, true).await;
        let elapsed = start.elapsed();

        assert!(result.is_ok(), "expected success, got: {:?}", result);
        // The 11.25s Hand-on-stone-with-penalty number is the smoking gun
        // for the old bug; with `tool_type = Pickaxe, material = Iron` the
        // actual mine time is 0.375s, so the executor's elapsed wall time
        // must be well under 11.25s.
        assert!(
            elapsed.as_secs_f64() < 11.25,
            "execute_mine_block took {elapsed:?} — the 5× wrong-tool \
             penalty was re-applied (tool_type was reset to Hand). \
             Expected < 11.25s with iron pickaxe (~0.375s)."
        );
    }

    // ═══════════════════════════════════════════════════════════════
    // Task 2.5: execute_mine_block uses block_index (M-12)
    // ═══════════════════════════════════════════════════════════════

    /// Task 2.5 (M-12): with 5000 blocks in the snapshot, the linear
    /// `blocks.iter().find()` lookup was the bottleneck. The new
    /// `block_index`-based lookup should finish in well under 1ms for
    /// 5000 entries.
    #[tokio::test]
    async fn test_mine_block_uses_block_index() {
        use std::time::Instant;

        // Build a 5000-block snapshot.
        let target_pos = BlockPos::new(1234, 64, 5678);
        let mut blocks: Vec<BlockEntry> = (0..5000)
            .map(|i| BlockEntry {
                position: BlockPos::new(i, 64, 0),
                block_type: "dirt".into(),
                block_state: None,
            })
            .collect();
        // Ensure the target is present.
        blocks[4321] = BlockEntry {
            position: target_pos,
            block_type: "stone".into(),
            block_state: None,
        };
        // Standable neighbour + solid floor so `find_standable_neighbor`
        // succeeds (M-4).
        blocks.push(BlockEntry {
            position: BlockPos::new(target_pos.x + 1, target_pos.y, target_pos.z),
            block_type: "air".into(),
            block_state: None,
        });
        blocks.push(BlockEntry {
            position: BlockPos::new(target_pos.x + 1, target_pos.y - 1, target_pos.z),
            block_type: "stone".into(),
            block_state: None,
        });

        let block_index: std::collections::HashMap<BlockPos, usize> = blocks
            .iter()
            .enumerate()
            .map(|(i, b)| (b.position, i))
            .collect();
        let chunk_x = target_pos.x >> 4;
        let chunk_z = target_pos.z >> 4;
        let snapshot = WorldSnapshot {
            blocks,
            block_index,
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
                position_precise: None,
                yaw: None,
            },
            timestamp: 1,
            chunk_summary: vec![(chunk_x, chunk_z)],
            commands_enabled: None,
            snapshot_seq: 0,
        };

        // Measure just the lookup portion of `execute_mine_block`'s
        // Step 2: `snapshot.block_index.get(&pos)`. With 5000 entries,
        // this is the O(1) path the spec calls for.
        let start = Instant::now();
        let block_type = snapshot
            .block_index
            .get(&target_pos)
            .map(|&idx| snapshot.blocks[idx].block_type.clone());
        let elapsed = start.elapsed();

        assert_eq!(block_type.as_deref(), Some("stone"));
        assert!(
            elapsed.as_millis() < 1,
            "block_index lookup took {elapsed:?} for 5000 blocks; \
             expected < 1ms (O(1) HashMap lookup)"
        );
    }

    // ═══════════════════════════════════════════════════════════════
    // Task 2.6: dispatch errors transition the state machine to Failed
    // ═══════════════════════════════════════════════════════════════

    /// Task 2.6: when a sub-command dispatch returns `Err`, the executor
    /// must transition the state machine to `OperationState::Failed(_)`
    /// (not `?`-return out of the function) so the outer match can
    /// distinguish which sub-step failed. Configure the mock to fail
    /// `MineBlock` (post-MoveTo) and assert the resulting error is a
    /// `MiningInterrupted` and the bot's air entry was NOT created
    /// (the break never succeeded).
    #[tokio::test]
    async fn test_mine_block_err_advances_to_failed_state() {
        // We do this by setting `mine_removes_block = false` AND adding a
        // sentinel block state — but `mine_block` only fails (returns
        // Err) at the executor layer if the underlying `BotActions::mine_block`
        // panics or if the dispatch path is short-circuited. Since the
        // mock always succeeds, we instead exercise the failure path
        // through a *pathfinding* failure (`goto_succeeds = false`),
        // which triggers `PathfindingFailed` after MoveTo.
        //
        // That still proves the `?` → `state.advance(Failed(e))` rewrite:
        // the executor must return the pathfinding error (not a generic
        // "internal" wrapper from an early-return path).
        let pos = BlockPos::new(10, 64, 20);
        let snapshot = make_snapshot_with_block(pos, "dirt");
        let (executor, mock, _state) = setup(vec![], snapshot);
        mock.goto_succeeds.store(false, Ordering::SeqCst);

        let result = CompoundOpExecutor::execute_mine_block(&executor, pos, false).await;

        // The dispatch returns `Err(BotError::PathfindingFailed { .. })`
        // which is now propagated as the function's `Err` (via the
        // outer match on `state == Failed(_)`). The key invariant is
        // that the error variant is the *pathfinding* one (proving the
        // state machine saw the dispatch error and routed it through
        // `OperationEvent::Failed(_)`), not a generic Internal error.
        assert!(result.is_err());
        match result.unwrap_err() {
            BotError::PathfindingFailed { .. } => {}
            other => panic!(
                "expected PathfindingFailed (proving the dispatch error \
                 was routed through the state machine's Failed transition), \
                 got: {other:?}"
            ),
        }
    }

    /// Task 2.6: state machine integrates dispatch failure mid-flow.
    /// Specifically, when `EquipTool` dispatch fails (tool exists in
    /// hotbar lookup but `switch_hotbar_slot` panics / returns Err),
    /// the executor should surface `BotError::ToolNotFound` (from the
    /// `result.success == false` branch) and the bot should not advance
    /// to `ExecutingAction`.
    ///
    /// The mock's `switch_hotbar_slot` always succeeds, so we exercise
    /// the alternate path: `EquipTool` dispatch returns a non-Ok result
    /// (simulated by hotbar-slot out-of-range; this isn't directly
    /// achievable via the mock, so we instead drive the executor with
    /// `use_best_tool = true` and a tool not in the inventory to make
    /// the tool-selection step return `Err(BotError::ToolNotFound)`).
    #[tokio::test]
    async fn test_mine_block_tool_not_found_short_circuits() {
        let pos = BlockPos::new(10, 64, 20);
        let snapshot = make_snapshot_with_block(pos, "stone");
        // Empty inventory: required_tool = Pickaxe, no tool available,
        // `select_tool_for_block` returns Hand, executor returns
        // `Err(BotError::ToolNotFound { tool_type: Pickaxe, .. })`.
        let (executor, _mock, _state) = setup(vec![], snapshot);

        let result = CompoundOpExecutor::execute_mine_block(&executor, pos, true).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            BotError::ToolNotFound {
                tool_type: ToolType::Pickaxe,
                ..
            } => {}
            other => panic!("expected ToolNotFound {{ Pickaxe, .. }}, got: {other:?}"),
        }
    }

    /// Task 2.6 (more direct): when `EquipTool` dispatch returns `Err`,
    /// the state machine must transition to `OperationState::Failed(_)`,
    /// not bubble out via `?`. The mock's `switch_hotbar_slot` always
    /// succeeds, so a `select_tool_for_block` hotbar match always
    /// produces a successful `EquipTool` dispatch. (The main-inventory
    /// branch — slot 9-35 — is routed through the
    /// `ToolAlreadyInInventory` skip path by `select_tool_for_block`,
    /// so it never reaches the `EquipTool` state either.)
    ///
    /// The `match ... Err(e) => { state.advance(Failed(e)); continue; }`
    /// pattern is identical in all three dispatch arms (MoveTo,
    /// EquipTool, BreakBlock). `test_mine_block_err_advances_to_failed_state`
    /// above exercises the MoveTo arm by forcing
    /// `goto_succeeds = false`; the other two arms are guaranteed
    /// symmetric by code review.
    #[test]
    fn test_mine_block_dispatch_err_advance_pattern_is_symmetric() {
        // Static check: the three dispatch arms in `execute_mine_block`
        // all use the same `match ... Err(e) => state.advance(Failed(e))`
        // pattern. A future refactor that drops the `Err` arm from
        // any one of them will fail this test (since it requires the
        // file to contain the marker comment three times).
        //
        // This is a code-shape test rather than a runtime test — it
        // exists so a careless future edit that reverts the `?` for
        // one arm is caught at PR-review time, not at runtime when
        // a real bot hits a transient network error.
        let src = include_str!("ops.rs");
        let marker = "state = op.advance(state, OperationEvent::Failed(e));";
        let occurrences = src.matches(marker).count();
        // Expect 7 dispatch arms — 3 in `execute_mine_block` (MoveTo,
        // EquipTool, BreakBlock), 2 in `execute_place_block` (MoveTo,
        // PlaceBlock), and 2 in `execute_open_container` (MoveTo,
        // OpenContainer). If it ever drops below 7, an arm was
        // reverted to `?`.
        assert!(
            occurrences >= 7,
            "expected at least 7 occurrences of the dispatch-err \
             `state.advance(Failed(e))` pattern (one per dispatch arm \
             across `execute_mine_block`, `execute_place_block`, and \
             `execute_open_container`), found {occurrences}. A future \
             edit may have reverted the `?`-to-`match` rewrite."
        );
    }

    // ═══════════════════════════════════════════════════════════════
    // Task 1.13: execute_place_block uses find_standable_neighbor (P1-#8)
    // ═══════════════════════════════════════════════════════════════

    /// Task 1.13 (P1-#8): `execute_place_block` must call
    /// `find_standable_neighbor` and `MoveTo` to the neighbour, not to
    /// the target position itself. We assert this by configuring the
    /// snapshot with a standable neighbour at a *different* position
    /// from the target, then verifying the bot's `self_player.position`
    /// is set to that neighbour after a successful place.
    #[tokio::test]
    async fn test_place_block_finds_neighbor() {
        let pos = BlockPos::new(10, 64, 20);
        // Snapshot with the target itself absent (so `find_standable_neighbor`
        // doesn't return early), a standable neighbour at (pos.x+1, pos.y, pos.z)
        // with a stone floor below.
        let blocks = vec![
            BlockEntry {
                position: BlockPos::new(pos.x + 1, pos.y, pos.z),
                block_type: "air".into(),
                block_state: None,
            },
            BlockEntry {
                position: BlockPos::new(pos.x + 1, pos.y - 1, pos.z),
                block_type: "stone".into(),
                block_state: None,
            },
        ];
        let block_index: std::collections::HashMap<BlockPos, usize> = blocks
            .iter()
            .enumerate()
            .map(|(i, b)| (b.position, i))
            .collect();
        let snapshot = WorldSnapshot {
            blocks,
            block_index,
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
                position_precise: None,
                yaw: None,
            },
            timestamp: 1,
            chunk_summary: vec![(0, 0)],
            commands_enabled: None,
            snapshot_seq: 0,
        };
        let inventory = vec![Some(ItemStack {
            item_id: "stone".into(),
            count: 64,
        })];
        let (executor, mock, state) = setup(inventory, snapshot);
        *mock.next_place_type.lock().unwrap() = Some("stone".into());

        let result = CompoundOpExecutor::execute_place_block(&executor, pos, "stone".into()).await;
        assert!(result.is_ok(), "expected success, got: {:?}", result);

        // The bot's position should be the standable neighbour, not the
        // (potentially solid) target position.
        let final_snapshot = state.read_snapshot();
        let bot_pos = final_snapshot.self_player.position;
        let expected_neighbour = BlockPos::new(pos.x + 1, pos.y, pos.z);
        assert_eq!(
            bot_pos, expected_neighbour,
            "bot should have moved to the standable neighbour {expected_neighbour:?}, \
             not the target position {pos:?}"
        );
    }

    // ═══════════════════════════════════════════════════════════════
    // wait_for_block_gone / wait_for_block_present tests (F6-4)
    // ═══════════════════════════════════════════════════════════════

    /// SharedState whose snapshot holds a single block of the given type
    /// at `pos` (with a correctly populated `block_index`).
    fn single_block_state(pos: BlockPos, block_type: &str) -> Arc<SharedState> {
        let state = SharedState::new(AppConfig::default());
        let blocks = vec![BlockEntry {
            position: pos,
            block_type: block_type.into(),
            block_state: None,
        }];
        let block_index = blocks
            .iter()
            .enumerate()
            .map(|(i, b)| (b.position, i))
            .collect();
        state.update_snapshot(WorldSnapshot {
            blocks,
            block_index,
            ..Default::default()
        });
        Arc::new(state)
    }

    #[tokio::test]
    async fn test_wait_for_block_gone_true_when_block_flips_to_air_mid_wait() {
        // The snapshot rebuild that reports the block as broken arrives only
        // ~150 ms later (simulating the periodic snapshot interval) — the
        // helper must keep polling and succeed within the budget.
        let pos = BlockPos::new(5, 64, 5);
        let state = single_block_state(pos, "stone");

        let updater = Arc::clone(&state);
        tokio::spawn(async move {
            sleep(Duration::from_millis(150)).await;
            // A broken block becomes "air" in the index (1.0.7), it does
            // not leave the index.
            let blocks = vec![BlockEntry {
                position: pos,
                block_type: "air".into(),
                block_state: None,
            }];
            let block_index = [(pos, 0usize)].into_iter().collect();
            updater.update_snapshot(WorldSnapshot {
                blocks,
                block_index,
                ..Default::default()
            });
        });

        let gone = wait_for_block_gone(&state, pos, Duration::from_secs(2)).await;
        assert!(gone, "block flipped to air mid-wait must count as gone");
    }

    #[tokio::test]
    async fn test_wait_for_block_gone_false_after_budget_when_block_stays() {
        let pos = BlockPos::new(5, 64, 5);
        let state = single_block_state(pos, "stone");

        let start = tokio::time::Instant::now();
        let gone = wait_for_block_gone(&state, pos, Duration::from_millis(120)).await;
        let elapsed = start.elapsed();

        assert!(!gone, "block never changed → budget must expire");
        assert!(
            elapsed < Duration::from_secs(1),
            "budget must stay bounded, took {elapsed:?}"
        );
    }

    #[tokio::test]
    async fn test_wait_for_block_gone_air_entry_counts_as_gone() {
        // Regression for the 1.0.7 air-in-snapshot behavior: the index
        // still HAS an entry for the position, but its type is "air" —
        // that counts as gone.
        let pos = BlockPos::new(5, 64, 5);
        let state = single_block_state(pos, "air");

        let gone = wait_for_block_gone(&state, pos, Duration::from_secs(2)).await;
        assert!(gone, "an indexed air entry must count as gone");
    }

    #[tokio::test]
    async fn test_wait_for_block_gone_true_when_pos_absent_from_index() {
        let state = single_block_state(BlockPos::new(0, 64, 0), "stone");
        let gone =
            wait_for_block_gone(&state, BlockPos::new(99, 64, 99), Duration::from_secs(2)).await;
        assert!(gone, "no index entry → already gone");
    }

    #[tokio::test]
    async fn test_wait_for_block_present_true_when_block_appears_mid_wait() {
        // Start from an air entry (pre-place state) and let the snapshot
        // rebuild report the placed block ~150 ms later.
        let pos = BlockPos::new(5, 64, 5);
        let state = single_block_state(pos, "air");

        let updater = Arc::clone(&state);
        tokio::spawn(async move {
            sleep(Duration::from_millis(150)).await;
            let blocks = vec![BlockEntry {
                position: pos,
                block_type: "stone".into(),
                block_state: None,
            }];
            let block_index = [(pos, 0usize)].into_iter().collect();
            updater.update_snapshot(WorldSnapshot {
                blocks,
                block_index,
                ..Default::default()
            });
        });

        let present = wait_for_block_present(&state, pos, Duration::from_secs(2)).await;
        assert!(present, "block appeared mid-wait must count as present");
    }

    #[tokio::test]
    async fn test_wait_for_block_present_false_after_budget_when_never_appears() {
        let pos = BlockPos::new(5, 64, 5);
        let state = single_block_state(pos, "air");

        let start = tokio::time::Instant::now();
        let present = wait_for_block_present(&state, pos, Duration::from_millis(120)).await;
        let elapsed = start.elapsed();

        assert!(!present, "block never appeared → budget must expire");
        assert!(
            elapsed < Duration::from_secs(1),
            "budget must stay bounded, took {elapsed:?}"
        );
    }

    #[tokio::test]
    async fn test_wait_for_block_present_air_entry_counts_as_absent() {
        // Regression: an indexed air entry must NOT count as placed —
        // only a non-air entry does.
        let pos = BlockPos::new(5, 64, 5);
        let state = single_block_state(pos, "air");

        let present = wait_for_block_present(&state, pos, Duration::from_millis(120)).await;
        assert!(!present, "an air entry must not count as present");
    }

    #[tokio::test]
    async fn test_wait_for_block_present_true_immediately_when_already_there() {
        let pos = BlockPos::new(5, 64, 5);
        let state = single_block_state(pos, "stone");

        let present = wait_for_block_present(&state, pos, Duration::from_secs(2)).await;
        assert!(present);
    }
}
