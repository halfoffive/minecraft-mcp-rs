//! Bot command implementations (move, dig, attack, interact).
//!
//! `CommandExecutor` receives [`BotCommand`]s from the MCP server via a
//! [`BotCommandReceiver`], dispatches them to the azalea [`Client`] API, and
//! sends a [`BotResult`] back through the oneshot channel.

use std::sync::Arc;
use std::time::Duration;

use azalea::pathfinder::goals::BlockPosGoal;
use azalea::prelude::*;
use tokio::time::sleep;
use tracing::{debug, trace, warn};

use crate::block_data::ItemStack;
use crate::bot::ops::CompoundOpExecutor;
use crate::channel::{BotCommandReceiver, BotCommandSender, ReceiverLease};
use crate::command_validate::clamp_to_i32;
use crate::error::BotError;
use crate::state::SharedState;
use crate::tool_select::find_tool_in_inventory;
use crate::types::{ActAction, ActResult, BlockPos, BotCommand, BotResult, Direction, GameMode};
use crate::utils::to_snake_case;

// ═══════════════════════════════════════════════════════════════
// BotActions trait — abstracts azalea Client for testability
// ═══════════════════════════════════════════════════════════════

/// Abstraction over azalea [`Client`] operations.
///
/// Each method maps to one bot action.  The real implementation delegates to
/// [`Client`]; a mock implementation records calls for unit tests.
#[allow(async_fn_in_trait)]
pub(crate) trait BotActions {
    /// Start pathfinding to a block position and await completion (or timeout).
    async fn goto(&self, pos: &BlockPos) -> Result<(), BotError>;

    /// Whether the bot is currently within the pathfinder's "arrived"
    /// radius of the most recent `goto` goal.
    ///
    /// Used by [`RealBotClient::goto`] as a 50ms fallback in case the
    /// tick handler's `notify_waiters()` is delayed or dropped — without
    /// it, a missed tick would force callers to wait the full
    /// `command_timeout_secs` before returning. Mock implementations
    /// return `true` once the pathfinder has been told to start.
    fn is_goto_target_reached(&self) -> bool;

    /// Perform a single jump.
    async fn jump(&self);

    /// Teleport by mutating the player's Position component.
    fn teleport(&self, pos: &BlockPos);

    /// Switch to a hotbar slot (0–8).
    fn switch_hotbar_slot(&self, slot: u8);

    /// Drop items from an inventory slot (0-35).
    fn drop_item(&self, slot: u8, count: u8);

    /// Start using the currently held item.
    fn start_use_item(&self);

    /// Send a chat message.
    fn chat(&self, message: &str);

    /// Attack an entity by its Minecraft entity ID.
    fn attack_entity(&self, entity_id: u32) -> Result<(), BotError>;

    /// Set crouching (shield block).
    fn set_crouching(&self, crouching: bool);

    /// Mine a block at the given position.
    fn mine_block(&self, pos: &BlockPos);

    /// Interact with a block (right-click).
    fn block_interact(&self, pos: &BlockPos);

    /// Open a container at the given position.
    ///
    /// On success the [`ContainerHandle`] is stored in [`SharedState`] so
    /// subsequent `take_from_container` / `put_into_container` / `close`
    /// commands can borrow it.
    async fn open_container(&self, pos: &BlockPos) -> Result<(), BotError>;

    /// Snapshot the player's inventory as a 36-slot vector.
    ///
    /// Index `0..=8` is the hotbar, `9..=35` is the main inventory. Empty
    /// slots are `None`. Used by [`CommandExecutor`] to answer
    /// [`BotCommand::QueryInventory`].
    fn inventory_entries(&self) -> Vec<Option<ItemStack>>;
}

// ═══════════════════════════════════════════════════════════════
// RealBotClient — delegates to azalea::Client
// ═══════════════════════════════════════════════════════════════

/// Wait for the pathfinder to reach the goal, with a 50ms position-check
/// fallback in case the tick handler's `notify_waiters()` is delayed or
/// dropped.
///
/// The fallback exists because the previous implementation waited
/// indefinitely on a single `notify` future; a missed tick would
/// deadlock the command until the full `timeout_dur` elapsed. This
/// loop races the notify against a 50ms timer that re-checks
/// `BotActions::is_goto_target_reached`, exiting early on either
/// success.
///
/// Extracted as a free function so the fallback semantics can be
/// unit-tested with a mock `BotActions` implementation (a real azalea
/// `Client` cannot be constructed in unit tests).
///
/// # Returns
/// - `Ok(())` if `is_goto_target_reached()` reports success within
///   `timeout_dur`, or if a `notify_waiters()` wakes the loop on a
///   tick where the position check passes.
/// - `Err(tokio::time::Duration)` with the elapsed time at the
///   deadline if neither signal fires within the window — the caller
///   is responsible for stopping the pathfinder and constructing the
///   user-facing `BotError::PathfindingFailed` (so the unit test can
///   assert on the helper in isolation).
pub(crate) async fn wait_for_goto_completion<B: BotActions>(
    bot: &B,
    notify: &std::sync::Arc<tokio::sync::Notify>,
    timeout_dur: Duration,
) -> Result<(), tokio::time::Duration> {
    let check_interval = Duration::from_millis(50);
    let start = tokio::time::Instant::now();
    loop {
        // Fast path: re-check the position before waiting so we exit
        // immediately if the bot arrived between the last tick and
        // our wake.
        if bot.is_goto_target_reached() {
            return Ok(());
        }
        let elapsed = start.elapsed();
        if elapsed >= timeout_dur {
            return Err(elapsed);
        }
        let remaining = timeout_dur - elapsed;
        let wait = std::cmp::min(remaining, check_interval);
        // Register the notified future *before* sleeping so a
        // `notify_waiters()` that fires while we are constructing the
        // future is not lost.
        let notified = notify.notified();
        tokio::pin!(notified);
        tokio::select! {
            _ = notified => {
                // Tick handler signalled a reach; the next loop
                // iteration's `is_goto_target_reached` call will
                // confirm and return Ok.
            }
            _ = tokio::time::sleep(wait) => {
                // Fallback: re-check position next iteration.
            }
        }
    }
}

/// Wraps an [`azalea::Client`] to implement [`BotActions`].
pub(crate) struct RealBotClient {
    client: Client,
    state: Arc<SharedState>,
    sender: BotCommandSender,
}

impl RealBotClient {
    pub fn new(client: Client, state: Arc<SharedState>, sender: BotCommandSender) -> Self {
        Self {
            client,
            state,
            sender,
        }
    }
}

impl BotActions for RealBotClient {
    fn is_goto_target_reached(&self) -> bool {
        // Delegate to azalea's pathfinder status. The method is sync on
        // the underlying `Client`; if a future azalea version moves it
        // behind an `await`, this becomes a fallible spot — wrap the
        // call in a `bool::from(...)` conversion rather than re-raising
        // the error so the `goto` fallback loop keeps its 50ms cadence.
        self.client.is_goto_target_reached()
    }

    async fn goto(&self, pos: &BlockPos) -> Result<(), BotError> {
        let az_pos = azalea::BlockPos::new(pos.x, pos.y, pos.z);
        let goal = BlockPosGoal(az_pos);

        // Honour the user-configured command timeout — read it through
        // the sender so the value is in lock-step with the timeout
        // `BotCommandSender::send_command` itself uses.
        let timeout_dur = self.sender.timeout();
        let timeout_secs = timeout_dur.as_secs();
        let notify = self.state.goto_notify();

        self.client.goto(goal).await;

        // Delegate the wait/fallback loop to a free function so the
        // 50ms fallback semantics can be unit-tested with a mock
        // `BotActions` implementation (real azalea `Client` cannot be
        // constructed in tests).
        match wait_for_goto_completion(self, &notify, timeout_dur).await {
            Ok(()) => Ok(()),
            Err(_elapsed) => {
                // Deadline elapsed — stop the pathfinder and report failure.
                self.client.stop_pathfinding();
                Err(BotError::PathfindingFailed {
                    target: BlockPos {
                        x: pos.x,
                        y: pos.y,
                        z: pos.z,
                    },
                    reason: format!("pathfinding timed out after {timeout_secs}s"),
                })
            }
        }
    }

    async fn jump(&self) {
        self.client.set_jumping(true);
        // A full Minecraft jump takes ~300ms from lift-off to landing; the
        // previous 100ms cut the jump short before the bot left the ground.
        sleep(Duration::from_millis(300)).await;
        self.client.set_jumping(false);
    }

    fn teleport(&self, pos: &BlockPos) {
        // Entity positions are continuous (block corners at integers); place
        // the player at the block centre on the XZ plane so they don't end up
        // straddling the north-west corner of the target block.
        let new_pos = azalea::entity::Position::new(azalea::Vec3 {
            x: pos.x as f64 + 0.5,
            y: pos.y as f64,
            z: pos.z as f64 + 0.5,
        });
        // Insert the new Position component on the player entity.
        // azalea 0.15.1 uses parking_lot::Mutex for ecs, so .lock() not .write().
        self.client
            .ecs
            .lock()
            .entity_mut(self.client.entity)
            .insert(new_pos);
    }

    fn switch_hotbar_slot(&self, slot: u8) {
        self.client.set_selected_hotbar_slot(slot);
    }

    fn drop_item(&self, slot: u8, count: u8) {
        // Best-effort: issue a `Throw` click on the player's inventory menu
        // (id=0, no container UI required). The Player menu places the hotbar
        // at slots 36..=44 and the main inventory at 9..=35, so the logical
        // inventory slot (0-35) is mapped to its menu slot. `ThrowClick::Single`
        // drops one item per click (like pressing Q); we issue `count` clicks.
        use azalea_inventory::operations::ThrowClick;

        // `set_selected_hotbar_slot` panics on slot > 8, and dropping from a
        // main-inventory slot (9-35) doesn't need selection, so only select
        // for hotbar slots. Save the currently selected slot first so it can
        // be restored after the drop (the selection here is a side effect of
        // targeting a hotbar slot, not an intentional user-facing change).
        let original_slot = self.client.selected_hotbar_slot();
        let switched_hotbar = slot <= 8;
        if switched_hotbar {
            self.client.set_selected_hotbar_slot(slot);
        }

        let menu_slot: u16 = if slot <= 8 {
            36 + slot as u16
        } else {
            slot as u16
        };
        let inventory = self.client.get_inventory();
        for _ in 0..count {
            inventory.click(ThrowClick::Single { slot: menu_slot });
        }

        // Restore the originally selected hotbar slot so the bot keeps holding
        // whatever it was holding before the drop. `selected_hotbar_slot()`
        // always returns 0..=8, so this never trips the >8 panic guard.
        if switched_hotbar {
            self.client.set_selected_hotbar_slot(original_slot);
        }
    }

    fn start_use_item(&self) {
        self.client.start_use_item();
    }

    fn chat(&self, message: &str) {
        self.client.chat(message);
    }

    fn attack_entity(&self, entity_id: u32) -> Result<(), BotError> {
        // azalea 0.15.1: entity_id_by_minecraft_id was renamed to
        // ecs_entity_by_minecraft_entity and takes a MinecraftEntityId.
        let entity = self
            .client
            .ecs_entity_by_minecraft_entity(azalea::world::MinecraftEntityId(entity_id as i32))
            .ok_or_else(|| BotError::Internal(format!("entity with id {} not found", entity_id)))?;
        self.client.attack(entity);
        Ok(())
    }

    fn set_crouching(&self, crouching: bool) {
        self.client.set_crouching(crouching);
    }

    fn mine_block(&self, pos: &BlockPos) {
        let az_pos = azalea::BlockPos::new(pos.x, pos.y, pos.z);
        self.client.start_mining(az_pos);
    }

    fn block_interact(&self, pos: &BlockPos) {
        let az_pos = azalea::BlockPos::new(pos.x, pos.y, pos.z);
        self.client.block_interact(az_pos);
    }

    async fn open_container(&self, pos: &BlockPos) -> Result<(), BotError> {
        let az_pos = azalea::BlockPos::new(pos.x, pos.y, pos.z);
        // open_container_at awaits the server confirming the container is open
        // (up to a 5s timeout) and returns a handle that auto-closes on drop.
        // Store it in SharedState so later container commands can borrow it.
        match self.client.open_container_at(az_pos).await {
            Some(handle) => {
                self.state.set_container_handle(Some(handle));
                Ok(())
            }
            None => Err(BotError::ContainerTimeout),
        }
    }

    fn inventory_entries(&self) -> Vec<Option<ItemStack>> {
        // The player inventory is the 36-slot `inventory` field of
        // `Menu::Player`. When a container is open the menu is no longer
        // `Player`, so fall back to an empty snapshot.
        let menu = self.client.menu();
        let player = match menu.try_as_player() {
            Some(p) => p,
            None => return Vec::new(),
        };
        // `player.inventory` is a `SlotList<36>` deref'ing to `[ItemStack; 36]`.
        player
            .inventory
            .iter()
            .map(|stack| {
                if stack.is_empty() {
                    None
                } else {
                    Some(ItemStack {
                        item_id: item_kind_to_id(stack.kind()),
                        count: stack.count().clamp(0, 255) as u8,
                    })
                }
            })
            .collect()
    }
}

// ═══════════════════════════════════════════════════════════════
// ItemKind → item_id string
// ═══════════════════════════════════════════════════════════════

/// Convert an azalea `ItemKind` (Debug variant name like `IronPickaxe`) into
/// the snake_case item id used by the block/tool tables (`iron_pickaxe`).
pub(crate) fn item_kind_to_id(kind: azalea::registry::builtin::ItemKind) -> String {
    to_snake_case(&format!("{kind:?}"))
}

// ═══════════════════════════════════════════════════════════════
// Direction → unit vector mapping
// ═══════════════════════════════════════════════════════════════

/// Map a [`Direction`] to a horizontal integer unit vector `(dx, dy, dz)`.
///
/// Returns `Some` for cardinal and diagonal directions (the y component is
/// always 0). Returns `None` for `Up`/`Down` because azalea's pathfinder
/// does not accept a purely vertical goal — callers should surface a clear
/// error for those.
fn direction_to_vector(dir: Direction) -> Option<(i32, i32, i32)> {
    match dir {
        Direction::North => Some((0, 0, -1)),
        Direction::South => Some((0, 0, 1)),
        Direction::East => Some((1, 0, 0)),
        Direction::West => Some((-1, 0, 0)),
        Direction::NorthEast => Some((1, 0, -1)),
        Direction::NorthWest => Some((-1, 0, -1)),
        Direction::SouthEast => Some((1, 0, 1)),
        Direction::SouthWest => Some((-1, 0, 1)),
        Direction::Up | Direction::Down => None,
    }
}

// ═══════════════════════════════════════════════════════════════
// CommandExecutor
// ═══════════════════════════════════════════════════════════════

/// Dispatches [`BotCommand`]s to an azalea client via [`BotActions`].
///
/// Owns the bot client, shared state, and (optionally) the command receiver
/// channel. Call [`run`](Self::run) to start the serial command processing
/// loop using the owned receiver, or [`run_with_lease`](Self::run_with_lease)
/// to drive the loop with a [`ReceiverLease`] that returns the receiver to
/// its slot when the executor is aborted.
pub(crate) struct CommandExecutor<B: BotActions> {
    bot: B,
    /// Shared state — `pub(crate)` so [`CompoundOpExecutor`] in `ops.rs` can
    /// read snapshots / check online status when driving compound operations
    /// via `&CommandExecutor` reference (sub-commands are dispatched directly
    /// through [`Self::dispatch`] rather than through the channel, to avoid
    /// re-entrant deadlock with `run_with_lease`).
    pub(crate) state: Arc<SharedState>,
    /// Owned receiver for the [`run`](Self::run) path. `None` when the
    /// executor was constructed via [`new_for_lease`](Self::new_for_lease).
    /// Only read by the test-only `run` method; `run_with_lease` uses the
    /// leased receiver instead.
    #[allow(dead_code)]
    receiver: Option<BotCommandReceiver>,
    /// Optional sender for issuing sub-commands.
    ///
    /// Historically `handle_act(Mine)` used this to delegate to
    /// [`CompoundOpExecutor`], which sent sub-commands through the same
    /// channel that `run_with_lease` consumes — causing a re-entrant
    /// deadlock. Compound operations now dispatch sub-commands directly via
    /// [`Self::dispatch`], so this field is no longer read by `handle_act`.
    /// It is retained because the connect chain still constructs it; removing
    /// it would require changing `new`, `new_for_lease`, `connect`, etc.
    #[allow(dead_code)]
    sender: Option<BotCommandSender>,
}

impl<B: BotActions> CommandExecutor<B> {
    /// Create a new executor that owns its receiver (used by tests).
    #[allow(dead_code)]
    pub fn new(
        bot: B,
        state: Arc<SharedState>,
        receiver: BotCommandReceiver,
        sender: Option<BotCommandSender>,
    ) -> Self {
        Self {
            bot,
            state,
            receiver: Some(receiver),
            sender,
        }
    }

    /// Create a new executor without an owned receiver; meant to be driven by
    /// [`run_with_lease`](Self::run_with_lease) so the receiver is returned to
    /// its shared slot when the task is aborted.
    pub(crate) fn new_for_lease(
        bot: B,
        state: Arc<SharedState>,
        sender: Option<BotCommandSender>,
    ) -> Self {
        Self {
            bot,
            state,
            receiver: None,
            sender,
        }
    }

    /// Run the command processing loop using the owned receiver.
    ///
    /// Receives commands one at a time from the channel, dispatches them,
    /// and sends a [`BotResult`] (or [`BotError`]) back via the oneshot
    /// responder.  Returns when all senders are dropped.
    ///
    /// # Panics
    ///
    /// Panics if the executor was constructed without an owned receiver
    /// (i.e. via [`new_for_lease`](Self::new_for_lease)).
    #[allow(dead_code)]
    pub async fn run(&mut self) {
        trace!("command executor loop started");

        // The receiver borrow is kept as a temporary inside the `while let`
        // condition so it does not extend into the loop body (where
        // `self.dispatch` needs `&self`). Binding it to a named local would
        // keep `self` mutably borrowed for the whole loop.
        while let Some(wrapped) = self
            .receiver
            .as_mut()
            .expect("CommandExecutor::run requires an owned receiver")
            .recv()
            .await
        {
            debug!(command = ?wrapped.command, "dispatching command");
            let result = self.dispatch(wrapped.command.clone()).await;
            if wrapped.respond_to.send(result).is_err() {
                warn!("command responder dropped — result lost");
            }
        }

        trace!("command executor loop ended (all senders dropped)");
    }

    /// Run the command processing loop using a [`ReceiverLease`].
    ///
    /// Unlike [`run`](Self::run), the receiver is not owned by the executor:
    /// it is borrowed from the shared slot via the lease. When the task is
    /// aborted (e.g. on disconnect), the lease drops and returns the receiver
    /// to the slot, allowing a future `Spawn` to re-acquire it.
    pub(crate) async fn run_with_lease(&mut self, mut lease: ReceiverLease) {
        trace!("command executor loop started (leased receiver)");

        loop {
            let wrapped = lease.receiver_mut().recv().await;
            match wrapped {
                Some(wrapped) => {
                    debug!(command = ?wrapped.command, "dispatching command");
                    let result = self.dispatch(wrapped.command.clone()).await;
                    if wrapped.respond_to.send(result).is_err() {
                        warn!("command responder dropped — result lost");
                    }
                }
                None => break,
            }
        }

        trace!("command executor loop ended (channel closed)");
    }

    /// Dispatch a single command and return the result.
    pub(crate) async fn dispatch(&self, cmd: BotCommand) -> Result<BotResult, BotError> {
        // Defense-in-depth: validate parameter bounds for every command before
        // execution. MCP handlers validate too, but this central gate catches
        // any handler that misses a bound (container slot/count, walk distance)
        // and covers commands generated internally by compound operations.
        crate::command_validate::validate_command(&cmd)?;

        // Check online status for commands that require a connection.
        if !self.state.is_online() {
            return Err(BotError::Offline("bot is not connected".into()));
        }

        match cmd {
            // ── Movement ──────────────────────────────────────────
            BotCommand::MoveTo(pos) => self.handle_move_to(pos).await,
            BotCommand::WalkDirection(dir, distance) => {
                self.handle_walk_direction(dir, distance).await
            }
            BotCommand::Jump => self.handle_jump().await,
            BotCommand::Teleport(pos) => self.handle_teleport(pos),

            // ── Block interaction ─────────────────────────────────
            BotCommand::BreakBlock(pos) => self.handle_break_block(pos),
            BotCommand::PlaceBlock(pos, block_type) => self.handle_place_block(pos, block_type),
            BotCommand::UseItemOnBlock(pos, item_slot) => {
                self.handle_use_item_on_block(pos, item_slot)
            }

            // ── Item / inventory ──────────────────────────────────
            BotCommand::SwitchHotbarSlot(slot) => self.handle_switch_hotbar_slot(slot),
            BotCommand::DropItem(slot, count) => self.handle_drop_item(slot, count),
            BotCommand::UseItem => self.handle_use_item(),
            BotCommand::UseItemWithSlot(slot) => self.handle_use_item_with_slot(slot),
            BotCommand::EquipTool(tool) => self.handle_equip_tool(tool, None),
            BotCommand::EquipToolWithMaterial(tool, material) => {
                self.handle_equip_tool(tool, Some(material))
            }

            // ── Container ─────────────────────────────────────────
            BotCommand::OpenContainer(pos) => self.handle_open_container(pos).await,
            BotCommand::TakeFromContainer(slot, count) => {
                self.handle_take_from_container(slot, count)
            }
            BotCommand::PutIntoContainer(slot, count) => {
                self.handle_put_into_container(slot, count)
            }
            BotCommand::CloseContainer => self.handle_close_container(),

            // ── Combat ────────────────────────────────────────────
            BotCommand::AttackEntity(id) => self.handle_attack_entity(id),
            BotCommand::ShieldBlock(blocking) => self.handle_shield_block(blocking),

            // ── Chat / command ────────────────────────────────────
            BotCommand::SendChat(msg) => self.handle_send_chat(msg),
            BotCommand::ExecuteCommand(cmd) => self.handle_execute_command(cmd),
            BotCommand::SetGameMode(mode) => self.handle_set_game_mode(mode),

            // ── Queries ───────────────────────────────────────────
            BotCommand::QueryNearbyBlocks(radius) => self.handle_query_nearby_blocks(radius),
            BotCommand::QueryNearbyEntities(radius) => self.handle_query_nearby_entities(radius),
            BotCommand::QuerySelfInfo => self.handle_query_self_info(),
            BotCommand::QueryInventory => self.handle_query_inventory(),
            BotCommand::QueryChunkSummary => self.handle_query_chunk_summary(),

            // ── v2 foundation: extended capabilities ──────────────
            BotCommand::SmartMove(target) => self.handle_smart_move(target).await,
            BotCommand::FlyTo(target) => self.handle_fly_to(target).await,
            BotCommand::CollectItems(radius) => self.handle_collect_items(radius).await,
            BotCommand::Act(action) => self.handle_act(action).await,
            BotCommand::QueryServerInfo => self.handle_query_server_info(),
            BotCommand::QueryChatHistory => self.handle_query_chat_history(),
            BotCommand::QueryWorldView(radius) => self.handle_query_world_view(radius),
        }
    }

    // ── Movement handlers ────────────────────────────────────────

    async fn handle_move_to(&self, pos: BlockPos) -> Result<BotResult, BotError> {
        trace!(?pos, "MoveTo");
        self.bot.goto(&pos).await?;

        // Verify the target was actually reached.
        if !self.state.is_online() {
            return Err(BotError::Offline("disconnected during movement".into()));
        }

        Ok(BotResult {
            success: true,
            message: format!("Moved to {}", pos),
            data: None,
        })
    }

    async fn handle_walk_direction(
        &self,
        dir: Direction,
        distance: u32,
    ) -> Result<BotResult, BotError> {
        trace!(?dir, distance, "WalkDirection");
        // For horizontal directions (cardinal + diagonal) translate the
        // request into a `MoveTo` at `current + unit_vector * distance` so the
        // pathfinder covers the exact block count. Vertical directions (Up/Down)
        // are not supported by azalea's pathfinder and surface a clear error.
        match direction_to_vector(dir) {
            Some((dx, dy, dz)) => {
                let current = self.state.read_snapshot().self_player.position;
                // Clamp to i32 range so a malicious or malformed `distance`
                // (u32 > i32::MAX) doesn't silently wrap to a negative
                // offset, which would make the bot walk in the opposite
                // direction.  Saturating add guards the coordinate
                // arithmetic against overflow from extreme inputs.
                let d = clamp_to_i32(distance);
                let target = BlockPos::new(
                    current.x.saturating_add(dx.saturating_mul(d)),
                    current.y.saturating_add(dy.saturating_mul(d)),
                    current.z.saturating_add(dz.saturating_mul(d)),
                );
                self.bot.goto(&target).await?;

                if !self.state.is_online() {
                    return Err(BotError::Offline("disconnected during movement".into()));
                }

                Ok(BotResult {
                    success: true,
                    message: format!("Walking {:?} for {} blocks", dir, distance),
                    data: None,
                })
            }
            None => {
                // Up/Down: azalea's pathfinder has no vertical-only goal.
                // `direction_to_vector` returns `None` for these, so surface a
                // clear error explaining the limitation.
                Err(BotError::Internal(format!(
                    "direction {dir:?} is not supported for distance move \
                     (vertical direction not supported for distance move); \
                     use MoveTo instead"
                )))
            }
        }
    }

    async fn handle_jump(&self) -> Result<BotResult, BotError> {
        trace!("Jump");
        self.bot.jump().await;
        Ok(BotResult {
            success: true,
            message: "Jumped".into(),
            data: None,
        })
    }

    fn handle_teleport(&self, pos: BlockPos) -> Result<BotResult, BotError> {
        trace!(?pos, "Teleport");
        self.bot.teleport(&pos);
        Ok(BotResult {
            success: true,
            message: format!("Teleported to {}", pos),
            data: None,
        })
    }

    // ── Block interaction handlers ───────────────────────────────

    fn handle_break_block(&self, pos: BlockPos) -> Result<BotResult, BotError> {
        trace!(?pos, "BreakBlock");
        // Chunk pre-check (P1-#7): previously this used
        // `snapshot.chunk_summary` to verify the chunk was loaded. That
        // failed in two common cases — (a) the chunk summary lags
        // behind actual chunk loads by one snapshot tick (M-2 packet
        // updates mark dirty blocks but the chunk-level summary
        // catches up on the next rebuild), and (b) any `BreakBlock`
        // that the bot itself caused by interacting with an unloaded
        // chunk edge (chunks within render distance are loaded but not
        // yet summarised). The block index is the source of truth
        // here: if a block at `pos` is present in the snapshot, the
        // chunk must be loaded enough for us to know about it. The
        // presence of the entry guarantees the bot has the chunk
        // data — anything not in the index is genuinely unknown.
        let snapshot = self.state.read_snapshot();
        if !snapshot.block_index.contains_key(&pos) {
            return Err(BotError::ChunkNotLoaded(pos));
        }
        self.bot.mine_block(&pos);
        Ok(BotResult {
            success: true,
            message: format!("Started mining block at {}", pos),
            data: None,
        })
    }

    fn handle_place_block(&self, pos: BlockPos, block_type: String) -> Result<BotResult, BotError> {
        trace!(?pos, %block_type, "PlaceBlock");
        // The MCP layer encodes the hotbar slot as "slot:N" in the block_type
        // field (see tools_block::handle_place_block). Select that slot before
        // right-clicking so the correct block is placed.
        if let Some(slot_str) = block_type.strip_prefix("slot:")
            && let Ok(slot) = slot_str.parse::<u8>()
        {
            if slot <= 8 {
                self.bot.switch_hotbar_slot(slot);
            } else {
                // Out-of-range slot — log but still attempt the interact.
                warn!(slot, "place_block slot out of hotbar range (0-8)");
            }
        }
        self.bot.block_interact(&pos);
        // Strip the internal "slot:N" prefix (if any) from the result
        // message so the LLM sees a clean block type name rather than
        // an opaque hotbar index like "Placed slot:3 at ...".
        let display_type = block_type.strip_prefix("slot:").unwrap_or(&block_type);
        Ok(BotResult {
            success: true,
            message: format!("Placed {} at {}", display_type, pos),
            data: None,
        })
    }

    fn handle_use_item_on_block(
        &self,
        pos: BlockPos,
        item_slot: Option<u8>,
    ) -> Result<BotResult, BotError> {
        trace!(?pos, ?item_slot, "UseItemOnBlock");
        // If a hotbar slot was specified, select it before interacting so the
        // correct item is used. Mirrors `handle_switch_hotbar_slot`'s range
        // check (the MCP layer also validates, but defend in depth).
        if let Some(slot) = item_slot
            && slot > 8
        {
            return Err(BotError::Internal(format!(
                "item_slot {slot} out of hotbar range (0-8)"
            )));
        }
        if let Some(slot) = item_slot {
            self.bot.switch_hotbar_slot(slot);
        }
        self.bot.block_interact(&pos);
        Ok(BotResult {
            success: true,
            message: format!("Used item on block at {} (slot: {:?})", pos, item_slot),
            data: None,
        })
    }

    // ── Item / inventory handlers ────────────────────────────────

    fn handle_switch_hotbar_slot(&self, slot: u8) -> Result<BotResult, BotError> {
        trace!(slot, "SwitchHotbarSlot");
        if slot > 8 {
            return Err(BotError::Internal(format!(
                "hotbar slot {} out of range (0-8)",
                slot
            )));
        }
        self.bot.switch_hotbar_slot(slot);
        Ok(BotResult {
            success: true,
            message: format!("Switched to hotbar slot {}", slot),
            data: None,
        })
    }

    fn handle_drop_item(&self, slot: u8, count: u8) -> Result<BotResult, BotError> {
        trace!(slot, count, "DropItem");
        // A count of 0 means "drop nothing" — return early without touching the
        // inventory or hotbar selection. Previously `drop_item` clamped the
        // loop bound to `count.max(1)`, which silently dropped a single item.
        if count == 0 {
            return Ok(BotResult {
                success: true,
                message: "Dropped 0 items".into(),
                data: None,
            });
        }
        self.bot.drop_item(slot, count);
        Ok(BotResult {
            success: true,
            message: format!("Dropped {} item(s) from slot {}", count, slot),
            data: None,
        })
    }

    fn handle_use_item(&self) -> Result<BotResult, BotError> {
        trace!("UseItem");
        self.bot.start_use_item();
        Ok(BotResult {
            success: true,
            message: "Started using item".into(),
            data: None,
        })
    }

    /// Atomically switch to a hotbar slot and use the held item.
    ///
    /// Both steps run within a single command dispatch so no other command
    /// can interleave between them (important under HTTP transport
    /// concurrency, where separate `SwitchHotbarSlot` + `UseItem` commands
    /// could be reordered or interleaved with other clients' commands).
    fn handle_use_item_with_slot(&self, slot: u8) -> Result<BotResult, BotError> {
        trace!(slot, "UseItemWithSlot");
        self.handle_switch_hotbar_slot(slot)?;
        self.handle_use_item()
    }

    fn handle_equip_tool(
        &self,
        tool: crate::types::ToolType,
        material: Option<crate::types::MaterialTier>,
    ) -> Result<BotResult, BotError> {
        trace!(?tool, ?material, "EquipTool");
        // `Hand` means "no specific tool needed" — nothing to equip.
        if tool == crate::types::ToolType::Hand {
            return Ok(BotResult {
                success: true,
                message: "No tool needed (Hand)".into(),
                data: None,
            });
        }

        // A material preference becomes a minimum harvest level: requesting
        // Diamond keeps diamond/netherite tools and rejects anything lower.
        let required_level = material.map(crate::block_data::harvest_level_of);

        // Search the inventory for a matching tool.
        let entries = self.bot.inventory_entries();
        match find_tool_in_inventory(&tool, &entries, required_level) {
            Some((_material, slot)) if slot <= 8 => {
                // Tool is in the hotbar — switch to it directly.
                self.bot.switch_hotbar_slot(slot);
                Ok(BotResult {
                    success: true,
                    message: format!("Equipped {tool:?} from hotbar slot {slot}"),
                    data: None,
                })
            }
            Some((_material, _slot)) => {
                // Tool exists but is in the main inventory (slot 9-35).
                // azalea's `set_selected_hotbar_slot` only accepts 0-8, so we
                // can't hotbar-select it directly. Moving items between the
                // main inventory and hotbar requires a container click flow
                // (deferred to a future version).
                Err(BotError::Internal(format!(
                    "{tool:?} found in main inventory but not in hotbar; \
                     move it to a hotbar slot first"
                )))
            }
            None => Err(BotError::ToolNotFound {
                tool_type: tool,
                material,
            }),
        }
    }

    // ── Container handlers ───────────────────────────────────────

    async fn handle_open_container(&self, pos: BlockPos) -> Result<BotResult, BotError> {
        trace!(?pos, "OpenContainer");
        // Reject if a container is already open to avoid leaking the previous
        // handle (azalea only supports one open container at a time).
        if self.state.has_container_open() {
            return Err(BotError::ContainerAlreadyOpen);
        }
        self.bot.open_container(&pos).await?;
        Ok(BotResult {
            success: true,
            message: format!("Opened container at {}", pos),
            data: None,
        })
    }

    fn handle_take_from_container(&self, slot: u8, count: u8) -> Result<BotResult, BotError> {
        trace!(slot, count, "TakeFromContainer");
        // Fail fast if the player inventory has no free slots. This prevents
        // shift-clicking a container item that would be dropped or lost.
        let snapshot = self.state.read_snapshot();
        if snapshot.self_player.inventory.len() >= 36 {
            return Err(BotError::InventoryFull);
        }
        drop(snapshot);
        // Best-effort: shift-click the given menu slot. For a container slot
        // this moves the whole stack into the player's inventory. `count` is
        // treated as a hint; partial moves require a pickup+place flow which
        // is deferred to a future version.
        let acted = self.state.with_container_handle(|handle| match handle {
            Some(handle) => {
                handle.shift_click(slot as usize);
                true
            }
            None => false,
        });
        if acted {
            Ok(BotResult {
                success: true,
                message: format!(
                    "Shift-clicked container slot {slot} (moved whole stack; count={count} is a hint)"
                ),
                data: None,
            })
        } else {
            Err(BotError::Internal("no container is currently open".into()))
        }
    }

    fn handle_put_into_container(&self, slot: u8, count: u8) -> Result<BotResult, BotError> {
        trace!(slot, count, "PutIntoContainer");
        // Best-effort: shift-click the given menu slot. When `slot` refers to
        // a player-inventory slot in the open menu, this moves the stack from
        // the player's inventory into the container. `count` is a hint; partial
        // moves require a pickup+place flow which is deferred to a future
        // version.
        let acted = self.state.with_container_handle(|handle| match handle {
            Some(handle) => {
                handle.shift_click(slot as usize);
                true
            }
            None => false,
        });
        if acted {
            Ok(BotResult {
                success: true,
                message: format!(
                    "Shift-clicked slot {slot} to move stack into the container (count={count} is a hint)"
                ),
                data: None,
            })
        } else {
            Err(BotError::Internal("no container is currently open".into()))
        }
    }

    fn handle_close_container(&self) -> Result<BotResult, BotError> {
        trace!("CloseContainer");
        // Container auto-closes when handle is dropped.
        self.state.set_container_handle(None);
        Ok(BotResult {
            success: true,
            message: "Container closed".into(),
            data: None,
        })
    }

    // ── Combat handlers ──────────────────────────────────────────

    fn handle_attack_entity(&self, entity_id: u32) -> Result<BotResult, BotError> {
        trace!(entity_id, "AttackEntity");
        // Fail fast if the target entity is outside a reasonable attack reach.
        // The snapshot may be slightly stale, so the threshold is generous.
        const MAX_ATTACK_REACH: f64 = 6.0;
        let snapshot = self.state.read_snapshot();
        if let Some(entity) = snapshot.entities.iter().find(|e| e.id == entity_id) {
            let dx = (entity.position.x - snapshot.self_player.position.x) as f64;
            let dy = (entity.position.y - snapshot.self_player.position.y) as f64;
            let dz = (entity.position.z - snapshot.self_player.position.z) as f64;
            let distance = (dx * dx + dy * dy + dz * dz).sqrt();
            if distance > MAX_ATTACK_REACH {
                return Err(BotError::TooFar {
                    target: entity.position,
                    current: snapshot.self_player.position,
                    max_distance: MAX_ATTACK_REACH,
                });
            }
        }
        drop(snapshot);
        self.bot.attack_entity(entity_id)?;
        Ok(BotResult {
            success: true,
            message: format!("Attacked entity {}", entity_id),
            data: None,
        })
    }

    fn handle_shield_block(&self, blocking: bool) -> Result<BotResult, BotError> {
        trace!(blocking, "ShieldBlock");
        // Crouching is used as a proxy for shield blocking in Minecraft.
        // `blocking = true` raises the shield (crouch); `false` lowers it.
        self.bot.set_crouching(blocking);
        Ok(BotResult {
            success: true,
            message: if blocking {
                "Shield raised (crouching)".into()
            } else {
                "Shield lowered (standing)".into()
            },
            data: None,
        })
    }

    // ── Chat / command handlers ──────────────────────────────────

    fn handle_send_chat(&self, msg: String) -> Result<BotResult, BotError> {
        trace!(%msg, "SendChat");
        self.bot.chat(&msg);
        Ok(BotResult {
            success: true,
            message: format!("Sent chat: {}", msg),
            data: None,
        })
    }

    fn handle_execute_command(&self, cmd: String) -> Result<BotResult, BotError> {
        trace!(%cmd, "ExecuteCommand");
        // The MCP layer (tools_chat::handle_execute_command) already
        // normalises the leading `/`, so `cmd` is passed straight to chat.
        // Re-prepending here would produce `//command`, which Minecraft
        // treats as a normal chat message rather than a command.
        self.bot.chat(&cmd);
        Ok(BotResult {
            success: true,
            message: format!("Executed command: {}", cmd),
            data: None,
        })
    }

    fn handle_set_game_mode(&self, mode: GameMode) -> Result<BotResult, BotError> {
        trace!(?mode, "SetGameMode");
        let mode_str = match mode {
            GameMode::Survival => "survival",
            GameMode::Creative => "creative",
            GameMode::Adventure => "adventure",
            GameMode::Spectator => "spectator",
        };
        // Sending `/gamemode` requires operator permissions. The server
        // rejects it silently (in chat) if the bot lacks OP, but azalea has
        // no way to detect that from the command path, so report success
        // honestly as "request sent" and flag the OP requirement.
        self.bot.chat(&format!("/gamemode {}", mode_str));
        Ok(BotResult {
            success: true,
            message: format!(
                "Requested game mode {:?} (requires OP; server may reject without operator permissions)",
                mode
            ),
            data: None,
        })
    }

    // ── Query handlers ───────────────────────────────────────────

    fn handle_query_nearby_blocks(&self, radius: u32) -> Result<BotResult, BotError> {
        trace!(radius, "QueryNearbyBlocks");
        let snapshot = self.state.read_snapshot();
        let pos = snapshot.self_player.position;
        let r = radius as i32;
        let blocks: Vec<_> = snapshot
            .blocks
            .iter()
            .filter(|b| {
                (b.position.x - pos.x).abs() <= r
                    && (b.position.y - pos.y).abs() <= r
                    && (b.position.z - pos.z).abs() <= r
            })
            .cloned()
            .collect();

        Ok(BotResult {
            success: true,
            message: format!("Found {} nearby blocks", blocks.len()),
            data: Some(serde_json::to_value(&blocks).unwrap_or_default()),
        })
    }

    fn handle_query_nearby_entities(&self, radius: u32) -> Result<BotResult, BotError> {
        trace!(radius, "QueryNearbyEntities");
        let snapshot = self.state.read_snapshot();
        let pos = snapshot.self_player.position;
        let r = radius as i32;
        let entities: Vec<_> = snapshot
            .entities
            .iter()
            .filter(|e| {
                (e.position.x - pos.x).abs() <= r
                    && (e.position.y - pos.y).abs() <= r
                    && (e.position.z - pos.z).abs() <= r
            })
            .cloned()
            .collect();

        Ok(BotResult {
            success: true,
            message: format!("Found {} nearby entities", entities.len()),
            data: Some(serde_json::to_value(&entities).unwrap_or_default()),
        })
    }

    fn handle_query_self_info(&self) -> Result<BotResult, BotError> {
        trace!("QuerySelfInfo");
        let snapshot = self.state.read_snapshot();
        Ok(BotResult {
            success: true,
            message: "Self info retrieved".into(),
            data: Some(serde_json::to_value(&snapshot.self_player).unwrap_or_default()),
        })
    }

    fn handle_query_inventory(&self) -> Result<BotResult, BotError> {
        trace!("QueryInventory");
        // Read the live inventory from the azalea client. The result is a
        // 36-element JSON array (index = slot, null = empty slot), matching
        // the format parsed by `compound_ops::query_inventory`.
        let entries = self.bot.inventory_entries();
        let arr: Vec<serde_json::Value> = entries
            .iter()
            .map(|opt| match opt {
                None => serde_json::Value::Null,
                Some(stack) => serde_json::json!({
                    "item_id": stack.item_id,
                    "count": stack.count,
                }),
            })
            .collect();
        let occupied = entries.iter().filter(|s| s.is_some()).count();
        Ok(BotResult {
            success: true,
            message: format!("Inventory has {occupied} occupied slot(s)"),
            data: Some(serde_json::Value::Array(arr)),
        })
    }

    fn handle_query_chunk_summary(&self) -> Result<BotResult, BotError> {
        trace!("QueryChunkSummary");
        let snapshot = self.state.read_snapshot();
        Ok(BotResult {
            success: true,
            message: format!("{} chunks loaded", snapshot.chunk_summary.len()),
            data: Some(serde_json::to_value(&snapshot.chunk_summary).unwrap_or_default()),
        })
    }

    // ── v2 foundation handlers ───────────────────────────────────

    /// Smart movement with auto-jump. Azalea's pathfinder already handles
    /// 1-block auto-jumps, so this delegates to [`BotActions::goto`] and
    /// inspects the result to report whether the target was reached or an
    /// obstacle blocked progress.
    async fn handle_smart_move(&self, target: BlockPos) -> Result<BotResult, BotError> {
        trace!(?target, "SmartMove");

        let goto_result = self.bot.goto(&target).await;
        let current_pos = self.state.read_snapshot().self_player.position;

        match goto_result {
            Ok(()) => {
                // Reached the target (or pathfinder believes it did).
                let reached = (current_pos.x - target.x).abs() <= 1
                    && (current_pos.y - target.y).abs() <= 1
                    && (current_pos.z - target.z).abs() <= 1;
                let reason = if reached { "reached" } else { "obstacle" };
                let obstacle = if reached {
                    None
                } else {
                    // Look for a solid block directly ahead (between current
                    // and target) to report as the obstacle.
                    find_obstacle_block(&self.state.read_snapshot(), current_pos, target)
                };

                Ok(BotResult {
                    success: true,
                    message: format!("SmartMove to {target}: {reason}"),
                    data: Some(serde_json::json!({
                        "reached": reached,
                        "reason": reason,
                        "position": [current_pos.x, current_pos.y, current_pos.z],
                        "obstacle": obstacle,
                    })),
                })
            }
            Err(e) => {
                // Pathfinder failed — treat as obstacle.
                let obstacle =
                    find_obstacle_block(&self.state.read_snapshot(), current_pos, target);
                Ok(BotResult {
                    success: true,
                    message: format!("SmartMove to {target} blocked: {e}"),
                    data: Some(serde_json::json!({
                        "reached": false,
                        "reason": "obstacle",
                        "position": [current_pos.x, current_pos.y, current_pos.z],
                        "obstacle": obstacle,
                    })),
                })
            }
        }
    }

    /// Creative-mode flight to a position. If the bot is not in creative mode,
    /// returns `not_creative`. Otherwise delegates to [`BotActions::goto`]
    /// (azalea's pathfinder can navigate 3D in creative flight).
    async fn handle_fly_to(&self, target: BlockPos) -> Result<BotResult, BotError> {
        trace!(?target, "FlyTo");
        let snapshot = self.state.read_snapshot();
        let gamemode = snapshot.self_player.gamemode;
        let current_pos = snapshot.self_player.position;

        if gamemode != GameMode::Creative {
            return Ok(BotResult {
                success: true,
                message: format!("FlyTo {target}: not in creative mode"),
                data: Some(serde_json::json!({
                    "reached": false,
                    "reason": "not_creative",
                    "position": [current_pos.x, current_pos.y, current_pos.z],
                })),
            });
        }

        // In creative mode the pathfinder can fly. Fall back to goto.
        let goto_result = self.bot.goto(&target).await;
        let final_pos = self.state.read_snapshot().self_player.position;

        let reached = (final_pos.x - target.x).abs() <= 1
            && (final_pos.y - target.y).abs() <= 1
            && (final_pos.z - target.z).abs() <= 1;
        let reason = if reached { "reached" } else { "obstacle" };

        let success = goto_result.is_ok() || reached;
        Ok(BotResult {
            success,
            message: format!("FlyTo {target}: {reason}"),
            data: Some(serde_json::json!({
                "reached": reached,
                "reason": reason,
                "position": [final_pos.x, final_pos.y, final_pos.z],
            })),
        })
    }

    /// Collect dropped item entities within `radius` blocks of the player.
    ///
    /// Walks to each item entity in turn; auto-pickup happens when the bot
    /// gets close. Returns the count of item entities visited.
    async fn handle_collect_items(&self, radius: u32) -> Result<BotResult, BotError> {
        trace!(radius, "CollectItems");
        let snapshot = self.state.read_snapshot();
        let player_pos = snapshot.self_player.position;
        let r = radius as i32;

        // Filter for item entities within radius. Entity types from azalea
        // for dropped items contain "item" (e.g. "item", "item_frame").
        // We match case-insensitively on "item" but exclude "item_frame"
        // since those are not pickup-able.
        let item_targets: Vec<BlockPos> = snapshot
            .entities
            .iter()
            .filter(|e| {
                let etype = e.entity_type.to_lowercase();
                etype == "item"
                    || etype == "item_entity"
                    || (etype.contains("item") && !etype.contains("frame"))
            })
            .filter(|e| {
                (e.position.x - player_pos.x).abs() <= r
                    && (e.position.y - player_pos.y).abs() <= r
                    && (e.position.z - player_pos.z).abs() <= r
            })
            .map(|e| e.position)
            .collect();

        if item_targets.is_empty() {
            return Ok(BotResult {
                success: true,
                message: "No items to collect".into(),
                data: Some(serde_json::json!({"visited": 0})),
            });
        }

        let mut visited: u32 = 0;
        for target in item_targets {
            // Walk to the item; auto-pickup occurs on proximity.
            if self.bot.goto(&target).await.is_ok() {
                // Brief pause for the server to process pickup.
                sleep(Duration::from_millis(200)).await;
                visited += 1;
            }
        }

        Ok(BotResult {
            success: true,
            message: format!(
                "Visited {visited} item drop location(s); auto-pickup expected on proximity"
            ),
            data: Some(serde_json::json!({"visited": visited})),
        })
    }

    /// Unified Act tool — dispatches the inner [`ActAction`] to the
    /// appropriate handler, then wraps the result in an [`ActResult`]
    /// enriched with nearby blocks/entities and self info from the snapshot.
    async fn handle_act(&self, action: ActAction) -> Result<BotResult, BotError> {
        trace!(?action, "Act");
        let (action_result, reason): (String, Option<String>) = match action {
            ActAction::Move { target } => match self.handle_move_to(target).await {
                Ok(r) => (r.message, None),
                Err(e) => ("failed".into(), Some(e.to_string())),
            },
            ActAction::SmartMove { target } => match self.handle_smart_move(target).await {
                Ok(r) => (r.message, None),
                Err(e) => ("failed".into(), Some(e.to_string())),
            },
            ActAction::Fly { target } => match self.handle_fly_to(target).await {
                Ok(r) => (r.message, None),
                Err(e) => ("failed".into(), Some(e.to_string())),
            },
            ActAction::Mine { block_pos } => {
                // Delegate to the compound operation executor which walks to
                // the block, selects the best tool, sleeps for the calculated
                // mine time, and verifies the block broke — returning a real
                // success/failure result instead of just "started mining".
                //
                // Sub-commands are dispatched directly via `&self` (this
                // `CommandExecutor`) rather than through the `BotCommandSender`
                // channel. The channel's only consumer is `run_with_lease`,
                // which is blocked awaiting this `dispatch` call to return —
                // sending sub-commands through the channel would deadlock
                // (30s timeout) waiting for a consumer that can never run.
                //
                // `Box::pin` is required because `dispatch` is recursive
                // through this call: `dispatch` → `handle_act` →
                // `execute_mine_block` → `query_inventory` → `dispatch`. Without
                // indirection the compiler cannot size the resulting future
                // (E0733). Boxing just this edge keeps the rest of `dispatch`
                // zero-cost.
                match Box::pin(CompoundOpExecutor::execute_mine_block(
                    self, block_pos, true,
                ))
                .await
                {
                    Ok(r) => (r.message, None),
                    Err(e) => ("failed".into(), Some(e.to_string())),
                }
            }
            ActAction::Attack { entity_id } => match self.handle_attack_entity(entity_id) {
                Ok(r) => (r.message, None),
                Err(e) => ("failed".into(), Some(e.to_string())),
            },
            ActAction::CollectItems { radius } => match self.handle_collect_items(radius).await {
                Ok(r) => (r.message, None),
                Err(e) => ("failed".into(), Some(e.to_string())),
            },
        };

        // Build the enriched result from the current snapshot.
        let snapshot = self.state.read_snapshot();
        let player_pos = snapshot.self_player.position;
        let perception_radius: i32 = self.state.read_config().block_perception_radius as i32;

        let nearby_blocks: Vec<_> = snapshot
            .blocks
            .iter()
            .filter(|b| {
                (b.position.x - player_pos.x).abs() <= perception_radius
                    && (b.position.y - player_pos.y).abs() <= perception_radius
                    && (b.position.z - player_pos.z).abs() <= perception_radius
            })
            .cloned()
            .collect();

        let nearby_entities: Vec<_> = snapshot
            .entities
            .iter()
            .filter(|e| {
                (e.position.x - player_pos.x).abs() <= perception_radius
                    && (e.position.y - player_pos.y).abs() <= perception_radius
                    && (e.position.z - player_pos.z).abs() <= perception_radius
            })
            .cloned()
            .collect();

        let act_result = ActResult {
            action_result,
            reason,
            nearby_blocks,
            nearby_entities,
            self_info: snapshot.self_player.clone(),
        };

        Ok(BotResult {
            success: act_result.reason.is_none(),
            message: "Act completed".into(),
            data: Some(serde_json::to_value(&act_result).unwrap_or_default()),
        })
    }

    /// Query server info — returns whether commands are enabled and the
    /// current gamemode, both read from the shared snapshot.
    fn handle_query_server_info(&self) -> Result<BotResult, BotError> {
        trace!("QueryServerInfo");
        let snapshot = self.state.read_snapshot();
        let gamemode = match snapshot.self_player.gamemode {
            GameMode::Survival => "survival",
            GameMode::Creative => "creative",
            GameMode::Adventure => "adventure",
            GameMode::Spectator => "spectator",
        };
        Ok(BotResult {
            success: true,
            message: format!(
                "commands_enabled={:?}, gamemode={gamemode}",
                snapshot.commands_enabled
            ),
            data: Some(serde_json::json!({
                "commands_enabled": snapshot.commands_enabled,
                "gamemode": gamemode,
            })),
        })
    }

    /// Query recent chat history from the shared state.
    fn handle_query_chat_history(&self) -> Result<BotResult, BotError> {
        trace!("QueryChatHistory");
        let messages = self.state.get_chat_messages();
        let arr: Vec<serde_json::Value> = messages
            .iter()
            .map(|(sender, message)| serde_json::json!({"sender": sender, "message": message}))
            .collect();
        Ok(BotResult {
            success: true,
            message: format!("{} recent chat message(s)", arr.len()),
            data: Some(serde_json::Value::Array(arr)),
        })
    }

    /// Query world view — returns a placeholder. The actual PNG rendering
    /// happens at the MCP tool layer which reads the snapshot directly.
    fn handle_query_world_view(&self, radius: u8) -> Result<BotResult, BotError> {
        trace!(radius, "QueryWorldView");
        Ok(BotResult {
            success: true,
            message: format!("World view radius {radius} (rendered at tool layer)"),
            data: Some(serde_json::json!({
                "radius": radius,
                "note": "rendered at tool layer",
            })),
        })
    }
}

// ═══════════════════════════════════════════════════════════════
// Free-function helpers for v2 handlers
// ═══════════════════════════════════════════════════════════════

/// Find the first solid block between `current` and `target` to report as
/// the obstacle that blocked movement. Returns the block type as a string,
/// or `None` if no candidate is found in the snapshot.
fn find_obstacle_block(
    snapshot: &crate::types::WorldSnapshot,
    current: BlockPos,
    target: BlockPos,
) -> Option<String> {
    // Walk the integer line from current toward target (XZ plane) and
    // return the first non-air block found in the snapshot. Interpolate both
    // axes proportionally so the scan follows the real line instead of a 45°
    // diagonal (which overshot one axis and skipped intermediate cells).
    let total_dx = target.x - current.x;
    let total_dz = target.z - current.z;
    let steps = total_dx.abs().max(total_dz.abs());
    if steps == 0 {
        return None;
    }
    for i in 1..=steps {
        let pos = BlockPos::new(
            current.x + total_dx * i / steps,
            current.y,
            current.z + total_dz * i / steps,
        );
        if let Some(&idx) = snapshot.block_index.get(&pos)
            && !snapshot.blocks[idx].block_type.is_empty()
            && snapshot.blocks[idx].block_type != "air"
        {
            return Some(snapshot.blocks[idx].block_type.clone());
        }
    }
    None
}

// ═══════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::{BotCommandSender, create_command_channel};
    use crate::config::AppConfig;
    use crate::types::{BlockEntry, EntityEntry, MaterialTier, SelfPlayer, ToolType};
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    // ═══════════════════════════════════════════════════════════════
    // MockBotClient
    // ═══════════════════════════════════════════════════════════════

    /// Tracks which methods were called and with what arguments.
    #[derive(Debug)]
    struct MockCallLog {
        goto_calls: Mutex<Vec<BlockPos>>,
        goto_succeeds: AtomicBool,
        /// When `true`, `is_goto_target_reached` always returns `false`,
        /// forcing `RealBotClient::goto`'s 50ms fallback loop to keep
        /// spinning until `command_timeout_secs` elapses. Default is
        /// `false` so the existing tests still see "arrived on first
        /// 50ms tick".
        goto_target_unreached: AtomicBool,
        jump_calls: AtomicUsize,
        teleport_calls: Mutex<Vec<BlockPos>>,
        hotbar_switch_calls: Mutex<Vec<u8>>,
        drop_item_calls: Mutex<Vec<(u8, u8)>>,
        use_item_calls: AtomicUsize,
        chat_calls: Mutex<Vec<String>>,
        attack_calls: Mutex<Vec<u32>>,
        attack_succeeds: AtomicBool,
        crouch_calls: Mutex<Vec<bool>>,
        mine_calls: Mutex<Vec<BlockPos>>,
        interact_calls: Mutex<Vec<BlockPos>>,
        container_open_calls: Mutex<Vec<BlockPos>>,
        inventory_calls: AtomicUsize,
        inventory: Mutex<Vec<Option<ItemStack>>>,
        position: Mutex<BlockPos>,
    }

    impl MockCallLog {
        fn new() -> Self {
            Self {
                goto_calls: Mutex::new(Vec::new()),
                goto_succeeds: AtomicBool::new(true),
                goto_target_unreached: AtomicBool::new(false),
                jump_calls: AtomicUsize::new(0),
                teleport_calls: Mutex::new(Vec::new()),
                hotbar_switch_calls: Mutex::new(Vec::new()),
                drop_item_calls: Mutex::new(Vec::new()),
                use_item_calls: AtomicUsize::new(0),
                chat_calls: Mutex::new(Vec::new()),
                attack_calls: Mutex::new(Vec::new()),
                attack_succeeds: AtomicBool::new(true),
                crouch_calls: Mutex::new(Vec::new()),
                mine_calls: Mutex::new(Vec::new()),
                interact_calls: Mutex::new(Vec::new()),
                container_open_calls: Mutex::new(Vec::new()),
                inventory_calls: AtomicUsize::new(0),
                inventory: Mutex::new(Vec::new()),
                position: Mutex::new(BlockPos::new(0, 64, 0)),
            }
        }
    }

    struct MockBotClient {
        log: Arc<MockCallLog>,
    }

    impl MockBotClient {
        fn new() -> Self {
            Self {
                log: Arc::new(MockCallLog::new()),
            }
        }

        fn log(&self) -> &Arc<MockCallLog> {
            &self.log
        }
    }

    impl BotActions for MockBotClient {
        fn is_goto_target_reached(&self) -> bool {
            // The mock defaults to "we have arrived" so the fallback
            // loop in `RealBotClient::goto` exits on the first 50ms
            // tick. Tests that need a delayed arrival (so the fallback
            // timer must actually be exercised) can flip
            // `goto_target_unreached` to `true` to make the position
            // check stay false for the whole `command_timeout_secs`
            // window.
            !self.log.goto_target_unreached.load(Ordering::SeqCst)
        }

        async fn goto(&self, pos: &BlockPos) -> Result<(), BotError> {
            self.log.goto_calls.lock().unwrap().push(*pos);
            if self.log.goto_succeeds.load(Ordering::SeqCst) {
                *self.log.position.lock().unwrap() = *pos;
                Ok(())
            } else {
                Err(BotError::PathfindingFailed {
                    target: BlockPos {
                        x: pos.x,
                        y: pos.y,
                        z: pos.z,
                    },
                    reason: "mock pathfinding failure".into(),
                })
            }
        }

        async fn jump(&self) {
            self.log.jump_calls.fetch_add(1, Ordering::SeqCst);
        }

        fn teleport(&self, pos: &BlockPos) {
            self.log.teleport_calls.lock().unwrap().push(*pos);
            *self.log.position.lock().unwrap() = *pos;
        }

        fn switch_hotbar_slot(&self, slot: u8) {
            self.log.hotbar_switch_calls.lock().unwrap().push(slot);
        }

        fn drop_item(&self, slot: u8, count: u8) {
            self.log.drop_item_calls.lock().unwrap().push((slot, count));
        }

        fn start_use_item(&self) {
            self.log.use_item_calls.fetch_add(1, Ordering::SeqCst);
        }

        fn chat(&self, message: &str) {
            self.log
                .chat_calls
                .lock()
                .unwrap()
                .push(message.to_string());
        }

        fn attack_entity(&self, entity_id: u32) -> Result<(), BotError> {
            self.log.attack_calls.lock().unwrap().push(entity_id);
            if self.log.attack_succeeds.load(Ordering::SeqCst) {
                Ok(())
            } else {
                Err(BotError::Internal("mock attack failure".into()))
            }
        }

        fn set_crouching(&self, crouching: bool) {
            self.log.crouch_calls.lock().unwrap().push(crouching);
        }

        fn mine_block(&self, pos: &BlockPos) {
            self.log.mine_calls.lock().unwrap().push(*pos);
        }

        fn block_interact(&self, pos: &BlockPos) {
            self.log.interact_calls.lock().unwrap().push(*pos);
        }

        async fn open_container(&self, pos: &BlockPos) -> Result<(), BotError> {
            self.log.container_open_calls.lock().unwrap().push(*pos);
            Ok(())
        }

        fn inventory_entries(&self) -> Vec<Option<ItemStack>> {
            self.log.inventory_calls.fetch_add(1, Ordering::SeqCst);
            self.log.inventory.lock().unwrap().clone()
        }
    }

    // ═══════════════════════════════════════════════════════════════
    // Helpers
    // ═══════════════════════════════════════════════════════════════

    fn make_executor() -> (
        CommandExecutor<MockBotClient>,
        BotCommandSender,
        Arc<SharedState>,
        Arc<MockCallLog>,
    ) {
        let config = AppConfig::default();
        let state = Arc::new(SharedState::new(config));
        let (sender, receiver) = create_command_channel(16, Arc::clone(&state));
        state.set_online(true);
        let mock = MockBotClient::new();
        let log = mock.log().clone();
        let executor = CommandExecutor::new(mock, Arc::clone(&state), receiver, None);
        (executor, sender, state, log)
    }

    async fn send_and_await(
        sender: &BotCommandSender,
        cmd: BotCommand,
    ) -> Result<BotResult, BotError> {
        sender.send_command(cmd).await
    }

    /// Spawn the executor's run loop in a background task.
    fn spawn_executor(mut executor: CommandExecutor<MockBotClient>) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            executor.run().await;
        })
    }

    /// Create a WorldSnapshot seeded with basic data for query tests.
    fn make_populated_snapshot(state: &SharedState) {
        let snap = crate::types::WorldSnapshot {
            blocks: vec![BlockEntry {
                position: BlockPos::new(5, 64, 0),
                block_type: "stone".into(),
                block_state: None,
            }],
            entities: vec![EntityEntry {
                id: 42,
                uuid: "test-entity".into(),
                entity_type: "zombie".into(),
                position: BlockPos::new(3, 64, 1),
                display_name: Some("Zombie".into()),
                health: Some(20.0),
            }],
            self_player: SelfPlayer {
                uuid: "player-uuid".into(),
                username: "TestBot".into(),
                position: BlockPos::new(0, 64, 0),
                health: 20.0,
                hunger: 20,
                gamemode: GameMode::Survival,
                held_item_slot: 0,
                inventory: Vec::new(),
            },
            timestamp: 1,
            chunk_summary: vec![(0, 0), (1, 0)],
            commands_enabled: None,
            ..Default::default()
        };
        state.update_snapshot(snap);
    }

    // ═══════════════════════════════════════════════════════════════
    // Construction tests
    // ═══════════════════════════════════════════════════════════════

    #[test]
    fn test_new_constructs() {
        let (_executor, _sender, _state, _log) = make_executor();
    }

    #[tokio::test]
    async fn test_run_loop_exits_when_sender_dropped() {
        let (executor, sender, _state, _log) = make_executor();
        let handle = spawn_executor(executor);

        // Send one command, then drop sender.
        let _ = send_and_await(&sender, BotCommand::Jump).await;
        drop(sender);

        // Executor should exit cleanly.
        handle.await.expect("executor should finish");
    }

    // ═══════════════════════════════════════════════════════════════
    // MoveTo tests
    // ═══════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_move_to_success() {
        let (executor, sender, _state, log) = make_executor();
        let handle = spawn_executor(executor);

        let pos = BlockPos::new(100, 64, 200);
        let result = send_and_await(&sender, BotCommand::MoveTo(pos)).await;

        assert!(result.is_ok(), "expected success, got: {:?}", result);
        let br = result.unwrap();
        assert!(br.success);
        assert!(br.message.contains("Moved to"));

        drop(sender);
        handle.await.expect("executor should finish");

        let goto_calls = log.goto_calls.lock().unwrap();
        assert_eq!(goto_calls.len(), 1);
        assert_eq!(goto_calls[0], pos);
    }

    #[tokio::test]
    async fn test_move_to_pathfinding_failed() {
        let (executor, sender, _state, log) = make_executor();
        // Configure mock to fail pathfinding.
        log.goto_succeeds.store(false, Ordering::SeqCst);
        let handle = spawn_executor(executor);

        let pos = BlockPos::new(999, 64, 999);
        let result = send_and_await(&sender, BotCommand::MoveTo(pos)).await;

        assert!(result.is_err(), "expected error, got: {:?}", result);
        assert!(matches!(result, Err(BotError::PathfindingFailed { .. })));

        drop(sender);
        handle.await.expect("executor should finish");

        // goto should still have been called.
        let goto_calls = log.goto_calls.lock().unwrap();
        assert_eq!(goto_calls.len(), 1);
    }

    /// Goto's 50ms fallback loop must re-check `is_goto_target_reached`
    /// even when no `notify_waiters()` is fired. This is the path
    /// exercised when the tick handler is delayed or missed — without
    /// it, a missed tick would force callers to wait the full
    /// `command_timeout_secs` before returning. The mock reports
    /// "target reached" (`goto_target_unreached = false`), so the
    /// first 50ms fallback tick should release the wait. We bound the
    /// test to well under `command_timeout_secs` (1s) to prove the
    /// fallback (and not the deadline) is what unblocks the call.
    #[tokio::test]
    async fn test_goto_falls_back_to_position_check() {
        // Construct a mock whose `is_goto_target_reached` returns
        // `true` (default `goto_target_unreached = false`).
        let mock = MockBotClient::new();
        let notify = Arc::new(tokio::sync::Notify::new());

        // Sanity check: the mock reports "arrived" so the fallback
        // loop's first iteration should return Ok immediately.
        assert!(mock.is_goto_target_reached());

        let result = wait_for_goto_completion(&mock, &notify, Duration::from_secs(5)).await;
        assert!(
            result.is_ok(),
            "expected Ok on first fallback tick, got: {:?}",
            result
        );
    }

    /// When the position check stays false for the full timeout window
    /// and the notify never fires, the fallback loop must still hit
    /// the deadline and return `Err(Duration)` — not hang forever.
    /// This is the "missed tick + unreachable target" worst case.
    #[tokio::test]
    async fn test_goto_falls_back_to_position_check_times_out() {
        let mock = MockBotClient::new();
        // Force the position check to always return false, so the
        // fallback loop is forced to spin until the timeout.
        mock.log.goto_target_unreached.store(true, Ordering::SeqCst);
        let notify = Arc::new(tokio::sync::Notify::new());

        let start = tokio::time::Instant::now();
        let result = wait_for_goto_completion(&mock, &notify, Duration::from_millis(200)).await;
        let elapsed = start.elapsed();

        assert!(
            result.is_err(),
            "expected Err (deadline) when position check stays false"
        );
        // The wait must not blow past the timeout by more than a
        // single 50ms fallback tick of scheduling slack.
        assert!(
            elapsed >= Duration::from_millis(200),
            "returned too early at {elapsed:?} (expected ≥ 200ms)"
        );
        assert!(
            elapsed < Duration::from_millis(400),
            "fallback loop is leaking past the deadline: {elapsed:?}"
        );
    }

    /// `notify_waiters()` from the tick handler must short-circuit the
    /// fallback loop even when the position check has not yet caught
    /// up. We simulate the race by setting `goto_target_unreached`
    /// initially, then having a background task that:
    /// 1. Fires the notify at 30ms (the "tick fired, position not yet
    ///    visible" race).
    /// 2. Flips `goto_target_unreached` to `false` at 60ms (the
    ///    "position now visible" update).
    ///
    /// The wait must return at or before 60ms (the position update),
    /// well under the 1s timeout. If the notify were ignored, the
    /// loop would sleep the full 50ms intervals and reach ~80ms; if
    /// the position update were ignored, the loop would hit the 1s
    /// deadline.
    #[tokio::test]
    async fn test_goto_notify_short_circuits_fallback() {
        let mock = MockBotClient::new();
        // Position check starts false.
        mock.log.goto_target_unreached.store(true, Ordering::SeqCst);
        let notify = Arc::new(tokio::sync::Notify::new());

        // Simulate the real race: tick handler fires notify first
        // (pathfinder reports done), then a moment later the snapshot
        // catches up and the position check would pass.
        let mock_log = Arc::clone(mock.log());
        let notify_for_task = Arc::clone(&notify);
        let firer = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(30)).await;
            notify_for_task.notify_waiters();
            tokio::time::sleep(Duration::from_millis(30)).await;
            mock_log
                .goto_target_unreached
                .store(false, Ordering::SeqCst);
        });

        // 1s timeout — the position flip (at ~60ms) must release us
        // first, not the deadline.
        let start = tokio::time::Instant::now();
        let result = wait_for_goto_completion(&mock, &notify, Duration::from_secs(1)).await;
        let elapsed = start.elapsed();
        firer.await.unwrap();

        assert!(
            result.is_ok(),
            "notify + position update must unblock fallback"
        );
        assert!(
            elapsed < Duration::from_millis(200),
            "wait_for_goto_completion should return promptly after position update, took {elapsed:?}"
        );
    }

    // ═══════════════════════════════════════════════════════════════
    // WalkDirection tests
    // ═══════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_walk_north() {
        // WalkDirection now routes horizontal moves through `goto` with a
        // target offset from the current position by the direction vector.
        let (executor, sender, state, log) = make_executor();
        make_populated_snapshot(&state);
        let handle = spawn_executor(executor);

        let result = send_and_await(&sender, BotCommand::WalkDirection(Direction::North, 1)).await;
        assert!(result.is_ok());
        assert!(result.unwrap().message.contains("Walking"));

        drop(sender);
        handle.await.expect("executor should finish");

        let goto_calls = log.goto_calls.lock().unwrap();
        assert_eq!(goto_calls.len(), 1);
        // Mock default position is (0, 64, 0); North is (0, 0, -1).
        assert_eq!(goto_calls[0], BlockPos::new(0, 64, -1));
    }

    #[tokio::test]
    async fn test_walk_south() {
        let (executor, sender, state, log) = make_executor();
        make_populated_snapshot(&state);
        let handle = spawn_executor(executor);

        let _ = send_and_await(&sender, BotCommand::WalkDirection(Direction::South, 1)).await;

        drop(sender);
        handle.await.expect("executor should finish");

        let goto_calls = log.goto_calls.lock().unwrap();
        assert_eq!(goto_calls.len(), 1);
        // South is (0, 0, +1).
        assert_eq!(goto_calls[0], BlockPos::new(0, 64, 1));
    }

    #[tokio::test]
    async fn test_walk_east() {
        let (executor, sender, state, log) = make_executor();
        make_populated_snapshot(&state);
        let handle = spawn_executor(executor);

        let _ = send_and_await(&sender, BotCommand::WalkDirection(Direction::East, 1)).await;

        drop(sender);
        handle.await.expect("executor should finish");

        let goto_calls = log.goto_calls.lock().unwrap();
        assert_eq!(goto_calls.len(), 1);
        // East is (+1, 0, 0).
        assert_eq!(goto_calls[0], BlockPos::new(1, 64, 0));
    }

    #[tokio::test]
    async fn test_walk_west() {
        let (executor, sender, state, log) = make_executor();
        make_populated_snapshot(&state);
        let handle = spawn_executor(executor);

        let _ = send_and_await(&sender, BotCommand::WalkDirection(Direction::West, 1)).await;

        drop(sender);
        handle.await.expect("executor should finish");

        let goto_calls = log.goto_calls.lock().unwrap();
        assert_eq!(goto_calls.len(), 1);
        // West is (-1, 0, 0).
        assert_eq!(goto_calls[0], BlockPos::new(-1, 64, 0));
    }

    #[tokio::test]
    async fn test_walk_diagonal_northeast() {
        // Diagonals are now supported via goto (unit vector combines x and z).
        let (executor, sender, state, log) = make_executor();
        make_populated_snapshot(&state);
        let handle = spawn_executor(executor);

        let _ = send_and_await(&sender, BotCommand::WalkDirection(Direction::NorthEast, 2)).await;

        drop(sender);
        handle.await.expect("executor should finish");

        let goto_calls = log.goto_calls.lock().unwrap();
        assert_eq!(goto_calls.len(), 1);
        // NorthEast is (+1, 0, -1); distance 2 → (2, 0, -2) offset.
        assert_eq!(goto_calls[0], BlockPos::new(2, 64, -2));
    }

    #[tokio::test]
    async fn test_walk_unsupported_direction() {
        // Up/Down cannot be translated to a horizontal goto target.
        let (executor, sender, _state, _log) = make_executor();
        let handle = spawn_executor(executor);

        let result = send_and_await(&sender, BotCommand::WalkDirection(Direction::Up, 1)).await;
        assert!(result.is_err());
        assert!(matches!(result, Err(BotError::Internal(_))));

        drop(sender);
        handle.await.expect("executor should finish");
    }

    // ═══════════════════════════════════════════════════════════════
    // Jump tests
    // ═══════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_jump() {
        let (executor, sender, _state, log) = make_executor();
        let handle = spawn_executor(executor);

        let result = send_and_await(&sender, BotCommand::Jump).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap().message, "Jumped");

        drop(sender);
        handle.await.expect("executor should finish");

        assert_eq!(log.jump_calls.load(Ordering::SeqCst), 1);
    }

    // ═══════════════════════════════════════════════════════════════
    // Teleport tests
    // ═══════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_teleport() {
        let (executor, sender, _state, log) = make_executor();
        let handle = spawn_executor(executor);

        let pos = BlockPos::new(50, 70, 100);
        let result = send_and_await(&sender, BotCommand::Teleport(pos)).await;
        assert!(result.is_ok());

        drop(sender);
        handle.await.expect("executor should finish");

        let tps = log.teleport_calls.lock().unwrap();
        assert_eq!(tps.len(), 1);
        assert_eq!(tps[0], pos);
    }

    // ═══════════════════════════════════════════════════════════════
    // SwitchHotbarSlot tests
    // ═══════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_switch_hotbar_slot() {
        let (executor, sender, _state, log) = make_executor();
        let handle = spawn_executor(executor);

        let result = send_and_await(&sender, BotCommand::SwitchHotbarSlot(4)).await;
        assert!(result.is_ok());

        drop(sender);
        handle.await.expect("executor should finish");

        let slots = log.hotbar_switch_calls.lock().unwrap();
        assert_eq!(slots.len(), 1);
        assert_eq!(slots[0], 4);
    }

    #[tokio::test]
    async fn test_switch_hotbar_slot_out_of_range() {
        let (executor, sender, _state, _log) = make_executor();
        let handle = spawn_executor(executor);

        let result = send_and_await(&sender, BotCommand::SwitchHotbarSlot(9)).await;
        assert!(result.is_err());

        drop(sender);
        handle.await.expect("executor should finish");
    }

    // ═══════════════════════════════════════════════════════════════
    // DropItem tests
    // ═══════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_drop_item() {
        let (executor, sender, _state, log) = make_executor();
        let handle = spawn_executor(executor);

        let result = send_and_await(&sender, BotCommand::DropItem(2, 5)).await;
        assert!(result.is_ok());

        drop(sender);
        handle.await.expect("executor should finish");

        let drops = log.drop_item_calls.lock().unwrap();
        assert_eq!(drops.len(), 1);
        assert_eq!(drops[0], (2, 5));
    }

    #[tokio::test]
    async fn test_drop_item_count_zero_rejected_by_validation() {
        // A count of 0 is rejected by the central validate_command gate in
        // dispatch (consistent with the MCP layer, which also rejects it), so
        // the bot action is never invoked.
        let (executor, sender, _state, log) = make_executor();
        let handle = spawn_executor(executor);

        let result = send_and_await(&sender, BotCommand::DropItem(2, 0)).await;
        assert!(matches!(result, Err(BotError::InvalidParams(_))));

        drop(sender);
        handle.await.expect("executor should finish");

        // The bot's drop_item must NOT have been called.
        let drops = log.drop_item_calls.lock().unwrap();
        assert!(
            drops.is_empty(),
            "expected no drop_item calls, got {:?}",
            drops
        );
    }

    // ═══════════════════════════════════════════════════════════════
    // UseItem tests
    // ═══════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_use_item() {
        let (executor, sender, _state, log) = make_executor();
        let handle = spawn_executor(executor);

        let result = send_and_await(&sender, BotCommand::UseItem).await;
        assert!(result.is_ok());

        drop(sender);
        handle.await.expect("executor should finish");

        assert_eq!(log.use_item_calls.load(Ordering::SeqCst), 1);
    }

    // ═══════════════════════════════════════════════════════════════
    // SendChat / ExecuteCommand / SetGameMode tests
    // ═══════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_send_chat() {
        let (executor, sender, _state, log) = make_executor();
        let handle = spawn_executor(executor);

        let result = send_and_await(&sender, BotCommand::SendChat("Hello world".into())).await;
        assert!(result.is_ok());

        drop(sender);
        handle.await.expect("executor should finish");

        let chats = log.chat_calls.lock().unwrap();
        assert_eq!(chats.len(), 1);
        assert_eq!(chats[0], "Hello world");
    }

    #[tokio::test]
    async fn test_execute_command() {
        // The MCP layer normalises the leading `/` before constructing
        // BotCommand::ExecuteCommand, so the executor passes the string
        // straight to chat without re-prepending.
        let (executor, sender, _state, log) = make_executor();
        let handle = spawn_executor(executor);

        let result =
            send_and_await(&sender, BotCommand::ExecuteCommand("/time set day".into())).await;
        assert!(result.is_ok());

        drop(sender);
        handle.await.expect("executor should finish");

        let chats = log.chat_calls.lock().unwrap();
        assert_eq!(chats.len(), 1);
        assert_eq!(chats[0], "/time set day");
    }

    #[tokio::test]
    async fn test_set_game_mode() {
        let (executor, sender, _state, log) = make_executor();
        let handle = spawn_executor(executor);

        let result = send_and_await(&sender, BotCommand::SetGameMode(GameMode::Creative)).await;
        assert!(result.is_ok());

        drop(sender);
        handle.await.expect("executor should finish");

        let chats = log.chat_calls.lock().unwrap();
        assert_eq!(chats.len(), 1);
        assert_eq!(chats[0], "/gamemode creative");
    }

    // ═══════════════════════════════════════════════════════════════
    // AttackEntity tests
    // ═══════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_attack_entity_success() {
        let (executor, sender, _state, log) = make_executor();
        let handle = spawn_executor(executor);

        let result = send_and_await(&sender, BotCommand::AttackEntity(42)).await;
        assert!(result.is_ok());

        drop(sender);
        handle.await.expect("executor should finish");

        let attacks = log.attack_calls.lock().unwrap();
        assert_eq!(attacks.len(), 1);
        assert_eq!(attacks[0], 42);
    }

    #[tokio::test]
    async fn test_attack_entity_failure() {
        let (executor, sender, _state, log) = make_executor();
        log.attack_succeeds.store(false, Ordering::SeqCst);
        let handle = spawn_executor(executor);

        let result = send_and_await(&sender, BotCommand::AttackEntity(99)).await;
        assert!(result.is_err());

        drop(sender);
        handle.await.expect("executor should finish");
    }

    // ═══════════════════════════════════════════════════════════════
    // ShieldBlock tests
    // ═══════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_shield_block() {
        let (executor, sender, _state, log) = make_executor();
        let handle = spawn_executor(executor);

        let result = send_and_await(&sender, BotCommand::ShieldBlock(true)).await;
        assert!(result.is_ok());
        assert!(result.unwrap().message.contains("Shield"));

        drop(sender);
        handle.await.expect("executor should finish");

        let crouches = log.crouch_calls.lock().unwrap();
        assert_eq!(crouches.len(), 1);
        assert!(crouches[0]); // crouching = true
    }

    #[tokio::test]
    async fn test_shield_block_lower() {
        // blocking=false should call set_crouching(false) and report lowering.
        let (executor, sender, _state, log) = make_executor();
        let handle = spawn_executor(executor);

        let result = send_and_await(&sender, BotCommand::ShieldBlock(false)).await;
        assert!(result.is_ok());
        let br = result.unwrap();
        assert!(br.message.contains("lowered"));

        drop(sender);
        handle.await.expect("executor should finish");

        let crouches = log.crouch_calls.lock().unwrap();
        assert_eq!(crouches.len(), 1);
        assert!(!crouches[0]); // crouching = false
    }

    // ═══════════════════════════════════════════════════════════════
    // BreakBlock / PlaceBlock / UseItemOnBlock tests
    // ═══════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_break_block() {
        let (executor, sender, state, log) = make_executor();
        let handle = spawn_executor(executor);

        // Seed the block index so the target block is considered
        // loaded. The chunk pre-check (P1-#7) now consults
        // `block_index.get(&pos)` rather than `chunk_summary`, so the
        // test must populate the index entry.
        let pos = BlockPos::new(10, 64, 20);
        state.update_snapshot(crate::types::WorldSnapshot {
            blocks: vec![BlockEntry {
                position: pos,
                block_type: "stone".into(),
                block_state: None,
            }],
            block_index: std::collections::HashMap::from([(pos, 0)]),
            ..Default::default()
        });

        let result = send_and_await(&sender, BotCommand::BreakBlock(pos)).await;
        assert!(result.is_ok());

        drop(sender);
        handle.await.expect("executor should finish");

        let mines = log.mine_calls.lock().unwrap();
        assert_eq!(mines.len(), 1);
        assert_eq!(mines[0], pos);
    }

    /// Chunk pre-check (P1-#7) regression: a `BreakBlock` must not be
    /// rejected with `ChunkNotLoaded` when the target block IS present
    /// in the snapshot — even if the chunk-summary is stale or empty.
    /// Previously the handler iterated `snapshot.chunk_summary`, which
    /// could miss blocks at the edge of the render distance or in
    /// chunks that the snapshot updater had not yet summarised, even
    /// though the bot's local chunk cache knew about them. The fix
    /// consults `snapshot.block_index` instead, which is the
    /// authoritative "do we know about this block" index.
    #[tokio::test]
    async fn test_break_block_loaded_chunk_not_rejected() {
        let (executor, sender, state, log) = make_executor();
        let handle = spawn_executor(executor);

        // Populate the snapshot with the target block but leave
        // `chunk_summary` empty (simulating the lag between chunk
        // load and summary rebuild). The block index entry is the
        // only thing the new pre-check looks at.
        let pos = BlockPos::new(20, 70, -5);
        state.update_snapshot(crate::types::WorldSnapshot {
            blocks: vec![BlockEntry {
                position: pos,
                block_type: "dirt".into(),
                block_state: None,
            }],
            // Empty chunk_summary would have caused the OLD pre-check
            // to return `ChunkNotLoaded`. The new pre-check must
            // accept this.
            chunk_summary: Vec::new(),
            block_index: std::collections::HashMap::from([(pos, 0)]),
            ..Default::default()
        });

        let result = send_and_await(&sender, BotCommand::BreakBlock(pos)).await;
        assert!(
            result.is_ok(),
            "BreakBlock at a block present in the snapshot must succeed \
             even when chunk_summary is empty (P1-#7 regression): {:?}",
            result
        );
        let br = result.unwrap();
        assert!(br.success);

        drop(sender);
        handle.await.expect("executor should finish");

        let mines = log.mine_calls.lock().unwrap();
        assert_eq!(mines.len(), 1);
        assert_eq!(mines[0], pos);
    }

    /// Sanity counterpart to `test_break_block_loaded_chunk_not_rejected`:
    /// when the block is genuinely unknown (no entry in the block
    /// index), the pre-check must still return `ChunkNotLoaded`. This
    /// guards against the new check becoming a no-op.
    #[tokio::test]
    async fn test_break_block_unknown_block_still_rejected() {
        let (executor, sender, state, _log) = make_executor();
        let handle = spawn_executor(executor);

        // Empty snapshot: nothing is loaded.
        state.update_snapshot(crate::types::WorldSnapshot::default());

        let pos = BlockPos::new(100, 64, 100);
        let result = send_and_await(&sender, BotCommand::BreakBlock(pos)).await;
        assert!(
            matches!(result, Err(BotError::ChunkNotLoaded(p)) if p == pos),
            "expected ChunkNotLoaded for an unknown block, got: {:?}",
            result
        );

        drop(sender);
        handle.await.expect("executor should finish");
    }

    #[tokio::test]
    async fn test_place_block() {
        let (executor, sender, _state, log) = make_executor();
        let handle = spawn_executor(executor);

        let pos = BlockPos::new(10, 64, 20);
        let result = send_and_await(&sender, BotCommand::PlaceBlock(pos, "stone".into())).await;
        assert!(result.is_ok());

        drop(sender);
        handle.await.expect("executor should finish");

        let interacts = log.interact_calls.lock().unwrap();
        assert_eq!(interacts.len(), 1);
        assert_eq!(interacts[0], pos);
        // No slot: prefix → no hotbar switch.
        assert!(log.hotbar_switch_calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_place_block_selects_slot_from_prefix() {
        // The MCP layer encodes the hotbar slot as "slot:N" in the block_type
        // field; the executor must select that slot before interacting.
        let (executor, sender, _state, log) = make_executor();
        let handle = spawn_executor(executor);

        let pos = BlockPos::new(10, 64, 20);
        let result = send_and_await(&sender, BotCommand::PlaceBlock(pos, "slot:3".into())).await;
        assert!(result.is_ok());

        drop(sender);
        handle.await.expect("executor should finish");

        let slots = log.hotbar_switch_calls.lock().unwrap();
        assert_eq!(slots.len(), 1);
        assert_eq!(slots[0], 3);

        let interacts = log.interact_calls.lock().unwrap();
        assert_eq!(interacts.len(), 1);
        assert_eq!(interacts[0], pos);
    }

    #[tokio::test]
    async fn test_use_item_on_block() {
        // Without an item_slot the bot interacts with the currently held item.
        let (executor, sender, _state, log) = make_executor();
        let handle = spawn_executor(executor);

        let pos = BlockPos::new(5, 65, 5);
        let result = send_and_await(&sender, BotCommand::UseItemOnBlock(pos, None)).await;
        assert!(result.is_ok());

        drop(sender);
        handle.await.expect("executor should finish");

        let interacts = log.interact_calls.lock().unwrap();
        assert_eq!(interacts.len(), 1);
        assert_eq!(interacts[0], pos);
        // No slot switching when item_slot is None.
        assert!(log.hotbar_switch_calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_use_item_on_block_with_slot() {
        // When item_slot is Some(n), the bot switches to slot n before
        // interacting.
        let (executor, sender, _state, log) = make_executor();
        let handle = spawn_executor(executor);

        let pos = BlockPos::new(5, 65, 5);
        let result = send_and_await(&sender, BotCommand::UseItemOnBlock(pos, Some(3))).await;
        assert!(result.is_ok());

        drop(sender);
        handle.await.expect("executor should finish");

        let slots = log.hotbar_switch_calls.lock().unwrap();
        assert_eq!(slots.len(), 1);
        assert_eq!(slots[0], 3);

        let interacts = log.interact_calls.lock().unwrap();
        assert_eq!(interacts.len(), 1);
        assert_eq!(interacts[0], pos);
    }

    // ═══════════════════════════════════════════════════════════════
    // Container tests
    // ═══════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_open_container() {
        let (executor, sender, _state, log) = make_executor();
        let handle = spawn_executor(executor);

        let pos = BlockPos::new(10, 64, 20);
        let result = send_and_await(&sender, BotCommand::OpenContainer(pos)).await;
        assert!(result.is_ok());

        drop(sender);
        handle.await.expect("executor should finish");

        let opens = log.container_open_calls.lock().unwrap();
        assert_eq!(opens.len(), 1);
        assert_eq!(opens[0], pos);
    }

    #[tokio::test]
    async fn test_close_container() {
        let (executor, sender, _state, _log) = make_executor();
        let handle = spawn_executor(executor);

        let result = send_and_await(&sender, BotCommand::CloseContainer).await;
        assert!(result.is_ok());

        drop(sender);
        handle.await.expect("executor should finish");
    }

    #[tokio::test]
    async fn test_take_from_container_no_container_open() {
        // Without a container handle in SharedState, the handler returns an
        // error instead of the old silent-success stub.
        let (executor, sender, _st, _log) = make_executor();
        let handle = spawn_executor(executor);

        let result = send_and_await(&sender, BotCommand::TakeFromContainer(3, 10)).await;
        assert!(result.is_err());
        assert!(matches!(result, Err(BotError::Internal(_))));

        drop(sender);
        handle.await.expect("executor should finish");
    }

    #[tokio::test]
    async fn test_put_into_container_no_container_open() {
        let (executor, sender, _state, _log) = make_executor();
        let handle = spawn_executor(executor);

        let result = send_and_await(&sender, BotCommand::PutIntoContainer(5, 8)).await;
        assert!(result.is_err());
        assert!(matches!(result, Err(BotError::Internal(_))));

        drop(sender);
        handle.await.expect("executor should finish");
    }

    // ═══════════════════════════════════════════════════════════════
    // EquipTool tests
    // ═══════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_equip_tool_not_found_with_empty_inventory() {
        // With an empty inventory, EquipTool returns ToolNotFound instead of
        // the old silent-success stub.
        let (executor, sender, _state, _log) = make_executor();
        let handle = spawn_executor(executor);

        let result = send_and_await(&sender, BotCommand::EquipTool(ToolType::Pickaxe)).await;
        assert!(result.is_err());
        assert!(matches!(result, Err(BotError::ToolNotFound { .. })));

        drop(sender);
        handle.await.expect("executor should finish");
    }

    #[tokio::test]
    async fn test_equip_tool_found_in_hotbar() {
        // With a pickaxe in hotbar slot 2, EquipTool selects slot 2.
        let (executor, sender, _state, log) = make_executor();
        // Seed the mock inventory: slot 2 has an iron_pickaxe.
        {
            let mut inv = log.inventory.lock().unwrap();
            inv.resize(9, None);
            inv[2] = Some(ItemStack {
                item_id: "iron_pickaxe".into(),
                count: 1,
            });
        }
        let handle = spawn_executor(executor);

        let result = send_and_await(&sender, BotCommand::EquipTool(ToolType::Pickaxe)).await;
        assert!(result.is_ok());
        let br = result.unwrap();
        assert!(br.message.contains("hotbar slot 2"));

        drop(sender);
        handle.await.expect("executor should finish");

        let slots = log.hotbar_switch_calls.lock().unwrap();
        assert_eq!(slots.len(), 1);
        assert_eq!(slots[0], 2);
    }

    #[tokio::test]
    async fn test_equip_tool_hand_is_noop() {
        let (executor, sender, _state, _log) = make_executor();
        let handle = spawn_executor(executor);

        let result = send_and_await(&sender, BotCommand::EquipTool(ToolType::Hand)).await;
        assert!(result.is_ok());
        let br = result.unwrap();
        assert!(br.message.contains("Hand"));

        drop(sender);
        handle.await.expect("executor should finish");
    }

    #[tokio::test]
    async fn test_equip_tool_with_material_accepts_meeting_tier() {
        // Iron pickaxe in hotbar slot 2; requesting an Iron minimum succeeds.
        let (executor, sender, _state, log) = make_executor();
        {
            let mut inv = log.inventory.lock().unwrap();
            inv.resize(9, None);
            inv[2] = Some(ItemStack {
                item_id: "iron_pickaxe".into(),
                count: 1,
            });
        }
        let handle = spawn_executor(executor);

        let result = send_and_await(
            &sender,
            BotCommand::EquipToolWithMaterial(ToolType::Pickaxe, MaterialTier::Iron),
        )
        .await;
        assert!(result.is_ok());

        drop(sender);
        handle.await.expect("executor should finish");
    }

    #[tokio::test]
    async fn test_equip_tool_with_material_rejects_below_preference() {
        // Only an iron pickaxe is available; requesting a Diamond minimum must
        // fail with ToolNotFound rather than silently equipping the iron one.
        let (executor, sender, _state, log) = make_executor();
        {
            let mut inv = log.inventory.lock().unwrap();
            inv.resize(9, None);
            inv[2] = Some(ItemStack {
                item_id: "iron_pickaxe".into(),
                count: 1,
            });
        }
        let handle = spawn_executor(executor);

        let result = send_and_await(
            &sender,
            BotCommand::EquipToolWithMaterial(ToolType::Pickaxe, MaterialTier::Diamond),
        )
        .await;
        assert!(matches!(result, Err(BotError::ToolNotFound { .. })));

        drop(sender);
        handle.await.expect("executor should finish");
    }

    // ═══════════════════════════════════════════════════════════════
    // dispatch validation gate (defense-in-depth) tests
    // ═══════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_dispatch_rejects_container_slot_over_max() {
        // slot 54 exceeds the schema max of 53 — the central validate_command
        // gate in dispatch must reject it as InvalidParams.
        let (executor, sender, _state, _log) = make_executor();
        let handle = spawn_executor(executor);

        let result = send_and_await(&sender, BotCommand::TakeFromContainer(54, 1)).await;
        assert!(matches!(result, Err(BotError::InvalidParams(_))));

        drop(sender);
        handle.await.expect("executor should finish");
    }

    #[tokio::test]
    async fn test_dispatch_rejects_container_count_over_max() {
        // count 65 exceeds the schema max of 64.
        let (executor, sender, _state, _log) = make_executor();
        let handle = spawn_executor(executor);

        let result = send_and_await(&sender, BotCommand::PutIntoContainer(0, 65)).await;
        assert!(matches!(result, Err(BotError::InvalidParams(_))));

        drop(sender);
        handle.await.expect("executor should finish");
    }

    #[tokio::test]
    async fn test_dispatch_rejects_walk_distance_over_max() {
        // distance 2000 exceeds the schema max of 1000.
        let (executor, sender, _state, _log) = make_executor();
        let handle = spawn_executor(executor);

        let result =
            send_and_await(&sender, BotCommand::WalkDirection(Direction::North, 2000)).await;
        assert!(matches!(result, Err(BotError::InvalidParams(_))));

        drop(sender);
        handle.await.expect("executor should finish");
    }

    #[test]
    fn test_find_obstacle_block_interpolates_line() {
        use std::collections::HashMap;
        // The obstacle at (1,64,5) lies on the true line from (0,64,0) to
        // (2,64,10). The old 45° diagonal scan visited (i,64,i) and would have
        // missed it; proportional interpolation must find it.
        let obstacle = BlockPos::new(1, 64, 5);
        let mut block_index = HashMap::new();
        block_index.insert(obstacle, 0usize);
        let snapshot = crate::types::WorldSnapshot {
            blocks: vec![BlockEntry {
                position: obstacle,
                block_type: "stone".into(),
                block_state: None,
            }],
            block_index,
            ..Default::default()
        };

        let found =
            find_obstacle_block(&snapshot, BlockPos::new(0, 64, 0), BlockPos::new(2, 64, 10));
        assert_eq!(found, Some("stone".to_string()));
    }

    // ═══════════════════════════════════════════════════════════════
    // Query tests
    // ═══════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_query_nearby_blocks() {
        let (executor, sender, state, _log) = make_executor();
        make_populated_snapshot(&state);
        let handle = spawn_executor(executor);

        let result = send_and_await(&sender, BotCommand::QueryNearbyBlocks(10)).await;
        assert!(result.is_ok());
        let br = result.unwrap();
        assert!(br.success);
        assert!(br.message.contains("Found 1"));

        drop(sender);
        handle.await.expect("executor should finish");
    }

    #[tokio::test]
    async fn test_query_nearby_blocks_empty() {
        let (executor, sender, _state, _log) = make_executor();
        // Don't populate — snapshot is empty.
        let handle = spawn_executor(executor);

        let result = send_and_await(&sender, BotCommand::QueryNearbyBlocks(10)).await;
        assert!(result.is_ok());
        let br = result.unwrap();
        assert!(br.message.contains("Found 0"));

        drop(sender);
        handle.await.expect("executor should finish");
    }

    #[tokio::test]
    async fn test_query_nearby_entities() {
        let (executor, sender, state, _log) = make_executor();
        make_populated_snapshot(&state);
        let handle = spawn_executor(executor);

        let result = send_and_await(&sender, BotCommand::QueryNearbyEntities(10)).await;
        assert!(result.is_ok());
        let br = result.unwrap();
        assert!(br.success);
        assert!(br.message.contains("Found 1"));

        drop(sender);
        handle.await.expect("executor should finish");
    }

    #[tokio::test]
    async fn test_query_self_info() {
        let (executor, sender, state, _log) = make_executor();
        make_populated_snapshot(&state);
        let handle = spawn_executor(executor);

        let result = send_and_await(&sender, BotCommand::QuerySelfInfo).await;
        assert!(result.is_ok());
        let br = result.unwrap();
        assert!(br.data.is_some());

        drop(sender);
        handle.await.expect("executor should finish");
    }

    #[tokio::test]
    async fn test_query_inventory() {
        let (executor, sender, _state, _log) = make_executor();
        let handle = spawn_executor(executor);

        let result = send_and_await(&sender, BotCommand::QueryInventory).await;
        assert!(result.is_ok());

        drop(sender);
        handle.await.expect("executor should finish");
    }

    #[tokio::test]
    async fn test_query_chunk_summary() {
        let (executor, sender, state, _log) = make_executor();
        make_populated_snapshot(&state);
        let handle = spawn_executor(executor);

        let result = send_and_await(&sender, BotCommand::QueryChunkSummary).await;
        assert!(result.is_ok());
        let br = result.unwrap();
        assert!(br.message.contains("2 chunks"));

        drop(sender);
        handle.await.expect("executor should finish");
    }

    // ═══════════════════════════════════════════════════════════════
    // Offline tests
    // ═══════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_command_while_offline_returns_error() {
        let (executor, sender, state, _log) = make_executor();
        state.set_online(false);
        let handle = spawn_executor(executor);

        let result = send_and_await(&sender, BotCommand::Jump).await;
        assert!(result.is_err());
        assert!(matches!(result, Err(BotError::Offline(_))));

        drop(sender);
        handle.await.expect("executor should finish");
    }

    #[tokio::test]
    async fn test_all_move_commands_offline() {
        let (executor, sender, state, _log) = make_executor();
        state.set_online(false);
        let handle = spawn_executor(executor);

        let cmds = vec![
            BotCommand::MoveTo(BlockPos::new(0, 0, 0)),
            BotCommand::WalkDirection(Direction::North, 1),
            BotCommand::Jump,
            BotCommand::Teleport(BlockPos::new(0, 0, 0)),
        ];

        for cmd in cmds {
            let result = send_and_await(&sender, cmd).await;
            assert!(
                matches!(result, Err(BotError::Offline(_))),
                "expected Offline, got: {:?}",
                result
            );
        }

        drop(sender);
        handle.await.expect("executor should finish");
    }

    // ═══════════════════════════════════════════════════════════════
    // Act (unified) tests
    // ═══════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_handle_act_returns_false_on_failure() {
        // When a sub-operation fails, `handle_act` must surface that in
        // `success: false` (previously hardcoded to `true`). The error is
        // captured in `ActResult::reason`, not propagated as `Err`.
        let (executor, sender, _state, log) = make_executor();
        log.attack_succeeds.store(false, Ordering::SeqCst);
        let handle = spawn_executor(executor);

        let result = send_and_await(
            &sender,
            BotCommand::Act(ActAction::Attack { entity_id: 99 }),
        )
        .await;
        assert!(result.is_ok(), "expected Ok, got: {:?}", result);
        let br = result.unwrap();
        assert!(
            !br.success,
            "expected success == false on sub-op failure, got success=true"
        );

        drop(sender);
        handle.await.expect("executor should finish");
    }

    // ═══════════════════════════════════════════════════════════════
    // Result format tests
    // ═══════════════════════════════════════════════════════════════

    #[test]
    fn test_bot_result_fields() {
        let result = BotResult {
            success: true,
            message: "test".into(),
            data: Some(serde_json::json!({"key": "value"})),
        };
        assert!(result.success);
        assert_eq!(result.message, "test");
        assert!(result.data.is_some());
    }

    // ═══════════════════════════════════════════════════════════════
    // Serial command processing test
    // ═══════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_serial_processing() {
        let (executor, sender, _state, log) = make_executor();
        let handle = spawn_executor(executor);

        // Send multiple commands.
        let s1 = sender.clone();
        let s2 = sender.clone();

        let h1 = tokio::spawn(async move { s1.send_command(BotCommand::Jump).await });
        let h2 = tokio::spawn(async move { s2.send_command(BotCommand::UseItem).await });

        let r1 = h1.await.unwrap();
        let r2 = h2.await.unwrap();
        assert!(r1.is_ok());
        assert!(r2.is_ok());

        drop(sender);
        handle.await.expect("executor should finish");

        assert_eq!(log.jump_calls.load(Ordering::SeqCst), 1);
        assert_eq!(log.use_item_calls.load(Ordering::SeqCst), 1);
    }

    // ═══════════════════════════════════════════════════════════════
    // Proptest — random positions for MoveTo
    // ═══════════════════════════════════════════════════════════════

    mod proptests {
        use super::*;
        use proptest::prelude::*;

        proptest! {
            #[test]
            fn test_dispatch_does_not_panic_with_any_position(x: i32, y: i32, z: i32) {
                let rt = tokio::runtime::Runtime::new().unwrap();
                rt.block_on(async {
                    let (executor, sender, _state, _log) = make_executor();
                    let handle = spawn_executor(executor);

                    let pos = BlockPos::new(x, y, z);
                    let result = send_and_await(&sender, BotCommand::MoveTo(pos)).await;
                    // Should not panic regardless of position.
                    let _ = result;

                    drop(sender);
                    handle.await.expect("executor should finish");
                });
            }

            #[test]
            fn test_switch_hotbar_valid_slot(slot in 0u8..=8u8) {
                let rt = tokio::runtime::Runtime::new().unwrap();
                rt.block_on(async {
                    let (executor, sender, _state, log) = make_executor();
                    let handle = spawn_executor(executor);

                    let _ = send_and_await(&sender, BotCommand::SwitchHotbarSlot(slot)).await;

                    drop(sender);
                    handle.await.expect("executor should finish");

                    let slots = log.hotbar_switch_calls.lock().unwrap();
                    assert_eq!(slots.len(), 1);
                    assert_eq!(slots[0], slot);
                });
            }
        }
    }
}
