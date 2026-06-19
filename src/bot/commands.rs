//! Bot command implementations (move, dig, attack, interact).
//!
//! [`CommandExecutor`] receives [`BotCommand`]s from the MCP server via a
//! [`BotCommandReceiver`], dispatches them to the azalea [`Client`] API, and
//! sends a [`BotResult`] back through the oneshot channel.
//!
//! > **Note:** Most types and functions in this module are scaffolding for
//! > the planned command-executor architecture and are not yet wired into
//! > the main event loop.  They are retained for the integration plan.

#![allow(dead_code)]

use std::sync::Arc;
use std::time::Duration;

use azalea::WalkDirection;
use azalea::pathfinder::goals::BlockPosGoal;
use azalea::prelude::*;
use tokio::time::{sleep, timeout};
use tracing::{debug, trace, warn};

use crate::block_data::ItemStack;
use crate::channel::{BotCommandReceiver, ReceiverLease};
use crate::error::BotError;
use crate::state::SharedState;
use crate::tool_select::find_tool_in_inventory;
use crate::types::{BlockPos, BotCommand, BotResult, Direction, GameMode};

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

    /// Start walking in a direction.
    fn walk(&self, direction: WalkDirection);

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

/// Wraps an [`azalea::Client`] to implement [`BotActions`].
pub(crate) struct RealBotClient {
    client: Client,
    state: Arc<SharedState>,
}

impl RealBotClient {
    pub fn new(client: Client, state: Arc<SharedState>) -> Self {
        Self { client, state }
    }
}

impl BotActions for RealBotClient {
    async fn goto(&self, pos: &BlockPos) -> Result<(), BotError> {
        let az_pos = azalea::BlockPos::new(pos.x, pos.y, pos.z);
        let goal = BlockPosGoal(az_pos);

        self.client.goto(goal).await;

        // Wait up to 30s for pathfinding to complete.
        let result = timeout(Duration::from_secs(30), async {
            loop {
                if self.client.is_goto_target_reached() {
                    return;
                }
                sleep(Duration::from_millis(100)).await;
            }
        })
        .await;

        match result {
            Ok(()) => Ok(()),
            Err(_) => {
                self.client.stop_pathfinding();
                Err(BotError::PathfindingFailed {
                    target: BlockPos {
                        x: pos.x,
                        y: pos.y,
                        z: pos.z,
                    },
                    reason: "pathfinding timed out after 30s".into(),
                })
            }
        }
    }

    fn walk(&self, direction: WalkDirection) {
        self.client.walk(direction);
    }

    async fn jump(&self) {
        self.client.set_jumping(true);
        sleep(Duration::from_millis(100)).await;
        self.client.set_jumping(false);
    }

    fn teleport(&self, pos: &BlockPos) {
        let new_pos = azalea::entity::Position::new(azalea::Vec3 {
            x: pos.x as f64,
            y: pos.y as f64,
            z: pos.z as f64,
        });
        // Insert the new Position component on the player entity.
        self.client
            .ecs
            .write()
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
        // for hotbar slots.
        if slot <= 8 {
            self.client.set_selected_hotbar_slot(slot);
        }

        let menu_slot: u16 = if slot <= 8 {
            36 + slot as u16
        } else {
            slot as u16
        };
        let inventory = self.client.get_inventory();
        for _ in 0..count.max(1) {
            inventory.click(ThrowClick::Single { slot: menu_slot });
        }
    }

    fn start_use_item(&self) {
        self.client.start_use_item();
    }

    fn chat(&self, message: &str) {
        self.client.chat(message);
    }

    fn attack_entity(&self, entity_id: u32) -> Result<(), BotError> {
        let entity = self
            .client
            .entity_id_by_minecraft_id(entity_id.into())
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
fn item_kind_to_id(kind: azalea::registry::builtin::ItemKind) -> String {
    to_snake_case(&format!("{kind:?}"))
}

/// Naive CamelCase → snake_case conversion.
///
/// Inserts `_` before each uppercase letter (except at the start) and
/// lowercases the result. Sufficient for azalea registry variant names.
fn to_snake_case(s: &str) -> String {
    let mut result = String::with_capacity(s.len() + 4);
    for (i, ch) in s.chars().enumerate() {
        if ch.is_uppercase() {
            if i > 0 {
                result.push('_');
            }
            result.push(ch.to_ascii_lowercase());
        } else {
            result.push(ch);
        }
    }
    result
}

// ═══════════════════════════════════════════════════════════════
// Direction → WalkDirection mapping
// ═══════════════════════════════════════════════════════════════

/// Map a cardinal [`Direction`] to an azalea [`WalkDirection`].
///
/// Returns `None` for unsupported directions (Up, Down, diagonals).
fn direction_to_walk(dir: Direction) -> Option<WalkDirection> {
    match dir {
        Direction::North => Some(WalkDirection::Forward),
        Direction::South => Some(WalkDirection::Backward),
        Direction::East => Some(WalkDirection::Right),
        Direction::West => Some(WalkDirection::Left),
        _ => None,
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
    state: Arc<SharedState>,
    /// Owned receiver for the [`run`] path. `None` when the executor was
    /// constructed via [`new_for_lease`](Self::new_for_lease).
    receiver: Option<BotCommandReceiver>,
}

impl<B: BotActions> CommandExecutor<B> {
    /// Create a new executor that owns its receiver (used by tests).
    pub fn new(bot: B, state: Arc<SharedState>, receiver: BotCommandReceiver) -> Self {
        Self {
            bot,
            state,
            receiver: Some(receiver),
        }
    }

    /// Create a new executor without an owned receiver; meant to be driven by
    /// [`run_with_lease`](Self::run_with_lease) so the receiver is returned to
    /// its shared slot when the task is aborted.
    pub(crate) fn new_for_lease(bot: B, state: Arc<SharedState>) -> Self {
        Self {
            bot,
            state,
            receiver: None,
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
        // Check online status for commands that require a connection.
        if !self.state.is_online() {
            return Err(BotError::Offline("bot is not connected".into()));
        }

        match cmd {
            // ── Movement ──────────────────────────────────────────
            BotCommand::MoveTo(pos) => self.handle_move_to(pos).await,
            BotCommand::WalkDirection(dir) => self.handle_walk_direction(dir),
            BotCommand::Jump => self.handle_jump().await,
            BotCommand::Teleport(pos) => self.handle_teleport(pos),

            // ── Block interaction ─────────────────────────────────
            BotCommand::BreakBlock(pos) => self.handle_break_block(pos),
            BotCommand::PlaceBlock(pos, block_type) => self.handle_place_block(pos, block_type),
            BotCommand::UseItemOnBlock(pos) => self.handle_use_item_on_block(pos),

            // ── Item / inventory ──────────────────────────────────
            BotCommand::SwitchHotbarSlot(slot) => self.handle_switch_hotbar_slot(slot),
            BotCommand::DropItem(slot, count) => self.handle_drop_item(slot, count),
            BotCommand::UseItem => self.handle_use_item(),
            BotCommand::EquipTool(tool) => self.handle_equip_tool(tool),

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
            BotCommand::ShieldBlock => self.handle_shield_block(),

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

    fn handle_walk_direction(&self, dir: Direction) -> Result<BotResult, BotError> {
        trace!(?dir, "WalkDirection");
        match direction_to_walk(dir) {
            Some(walk_dir) => {
                self.bot.walk(walk_dir);
                Ok(BotResult {
                    success: true,
                    message: format!("Walking {:?}", dir),
                    data: None,
                })
            }
            None => Err(BotError::Internal(format!(
                "direction {:?} is not supported for walk; use MoveTo instead",
                dir
            ))),
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
        Ok(BotResult {
            success: true,
            message: format!("Placed {} at {}", block_type, pos),
            data: None,
        })
    }

    fn handle_use_item_on_block(&self, pos: BlockPos) -> Result<BotResult, BotError> {
        trace!(?pos, "UseItemOnBlock");
        self.bot.block_interact(&pos);
        Ok(BotResult {
            success: true,
            message: format!("Used item on block at {}", pos),
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

    fn handle_equip_tool(&self, tool: crate::types::ToolType) -> Result<BotResult, BotError> {
        trace!(?tool, "EquipTool");
        // `Hand` means "no specific tool needed" — nothing to equip.
        if tool == crate::types::ToolType::Hand {
            return Ok(BotResult {
                success: true,
                message: "No tool needed (Hand)".into(),
                data: None,
            });
        }

        // Search the inventory for a matching tool.
        let entries = self.bot.inventory_entries();
        match find_tool_in_inventory(&tool, &entries) {
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
                material: None,
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
        self.bot.attack_entity(entity_id)?;
        Ok(BotResult {
            success: true,
            message: format!("Attacked entity {}", entity_id),
            data: None,
        })
    }

    fn handle_shield_block(&self) -> Result<BotResult, BotError> {
        trace!("ShieldBlock");
        // Crouching is used as a proxy for shield blocking in Minecraft.
        self.bot.set_crouching(true);
        Ok(BotResult {
            success: true,
            message: "Shield raised (crouching)".into(),
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
}

// ═══════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::{BotCommandSender, create_command_channel};
    use crate::config::AppConfig;
    use crate::types::{BlockEntry, EntityEntry, SelfPlayer, ToolType};
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
        walk_calls: Mutex<Vec<WalkDirection>>,
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
                walk_calls: Mutex::new(Vec::new()),
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

        fn walk(&self, direction: WalkDirection) {
            self.log.walk_calls.lock().unwrap().push(direction);
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
        let (sender, receiver) = create_command_channel(16);
        let config = AppConfig::default();
        let state = Arc::new(SharedState::new(config));
        state.set_online(true);
        let mock = MockBotClient::new();
        let log = mock.log().clone();
        let executor = CommandExecutor::new(mock, Arc::clone(&state), receiver);
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
            },
            timestamp: 1,
            chunk_summary: vec![(0, 0), (1, 0)],
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

    // ═══════════════════════════════════════════════════════════════
    // WalkDirection tests
    // ═══════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_walk_north() {
        let (executor, sender, _state, log) = make_executor();
        let handle = spawn_executor(executor);

        let result = send_and_await(&sender, BotCommand::WalkDirection(Direction::North)).await;
        assert!(result.is_ok());
        assert!(result.unwrap().message.contains("Walking"));

        drop(sender);
        handle.await.expect("executor should finish");

        let walks = log.walk_calls.lock().unwrap();
        assert_eq!(walks.len(), 1);
        assert_eq!(walks[0], WalkDirection::Forward);
    }

    #[tokio::test]
    async fn test_walk_south() {
        let (executor, sender, _state, log) = make_executor();
        let handle = spawn_executor(executor);

        let _ = send_and_await(&sender, BotCommand::WalkDirection(Direction::South)).await;

        drop(sender);
        handle.await.expect("executor should finish");

        let walks = log.walk_calls.lock().unwrap();
        assert_eq!(walks[0], WalkDirection::Backward);
    }

    #[tokio::test]
    async fn test_walk_east() {
        let (executor, sender, _state, log) = make_executor();
        let handle = spawn_executor(executor);

        let _ = send_and_await(&sender, BotCommand::WalkDirection(Direction::East)).await;

        drop(sender);
        handle.await.expect("executor should finish");

        let walks = log.walk_calls.lock().unwrap();
        assert_eq!(walks[0], WalkDirection::Right);
    }

    #[tokio::test]
    async fn test_walk_west() {
        let (executor, sender, _state, log) = make_executor();
        let handle = spawn_executor(executor);

        let _ = send_and_await(&sender, BotCommand::WalkDirection(Direction::West)).await;

        drop(sender);
        handle.await.expect("executor should finish");

        let walks = log.walk_calls.lock().unwrap();
        assert_eq!(walks[0], WalkDirection::Left);
    }

    #[tokio::test]
    async fn test_walk_unsupported_direction() {
        let (executor, sender, _state, _log) = make_executor();
        let handle = spawn_executor(executor);

        let result = send_and_await(&sender, BotCommand::WalkDirection(Direction::Up)).await;
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

        let result = send_and_await(&sender, BotCommand::ShieldBlock).await;
        assert!(result.is_ok());
        assert!(result.unwrap().message.contains("Shield"));

        drop(sender);
        handle.await.expect("executor should finish");

        let crouches = log.crouch_calls.lock().unwrap();
        assert_eq!(crouches.len(), 1);
        assert!(crouches[0]); // crouching = true
    }

    // ═══════════════════════════════════════════════════════════════
    // BreakBlock / PlaceBlock / UseItemOnBlock tests
    // ═══════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_break_block() {
        let (executor, sender, _state, log) = make_executor();
        let handle = spawn_executor(executor);

        let pos = BlockPos::new(10, 64, 20);
        let result = send_and_await(&sender, BotCommand::BreakBlock(pos)).await;
        assert!(result.is_ok());

        drop(sender);
        handle.await.expect("executor should finish");

        let mines = log.mine_calls.lock().unwrap();
        assert_eq!(mines.len(), 1);
        assert_eq!(mines[0], pos);
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
        let (executor, sender, _state, log) = make_executor();
        let handle = spawn_executor(executor);

        let pos = BlockPos::new(5, 65, 5);
        let result = send_and_await(&sender, BotCommand::UseItemOnBlock(pos)).await;
        assert!(result.is_ok());

        drop(sender);
        handle.await.expect("executor should finish");

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
            BotCommand::WalkDirection(Direction::North),
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
    // Direction mapping tests
    // ═══════════════════════════════════════════════════════════════

    #[test]
    fn test_direction_to_walk_cardinals() {
        assert_eq!(
            direction_to_walk(Direction::North),
            Some(WalkDirection::Forward)
        );
        assert_eq!(
            direction_to_walk(Direction::South),
            Some(WalkDirection::Backward)
        );
        assert_eq!(
            direction_to_walk(Direction::East),
            Some(WalkDirection::Right)
        );
        assert_eq!(
            direction_to_walk(Direction::West),
            Some(WalkDirection::Left)
        );
    }

    #[test]
    fn test_direction_to_walk_unsupported() {
        assert_eq!(direction_to_walk(Direction::Up), None);
        assert_eq!(direction_to_walk(Direction::Down), None);
        assert_eq!(direction_to_walk(Direction::NorthEast), None);
        assert_eq!(direction_to_walk(Direction::NorthWest), None);
        assert_eq!(direction_to_walk(Direction::SouthEast), None);
        assert_eq!(direction_to_walk(Direction::SouthWest), None);
    }

    #[test]
    fn test_direction_to_walk_exhaustive() {
        // All 10 Direction variants are handled.
        let all = [
            Direction::North,
            Direction::South,
            Direction::East,
            Direction::West,
            Direction::Up,
            Direction::Down,
            Direction::NorthEast,
            Direction::NorthWest,
            Direction::SouthEast,
            Direction::SouthWest,
        ];
        for dir in all {
            let _ = direction_to_walk(dir); // no panic
        }
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
