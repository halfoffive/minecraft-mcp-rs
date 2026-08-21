//! Bot command implementations (move, dig, attack, interact).
//!
//! `CommandExecutor` receives [`BotCommand`]s from the MCP server via a
//! [`BotCommandReceiver`], dispatches them to the azalea [`Client`] API, and
//! sends a [`BotResult`] back through the oneshot channel.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use azalea::pathfinder::goals::BlockPosGoal;
use azalea::prelude::*;
use tokio::time::sleep;
use tracing::{debug, trace, warn};

use crate::block_data::ItemStack;
use crate::bot::ops::{CompoundOpExecutor, wait_for_block_present};
use crate::channel::{
    BotCommandReceiver, BotCommandSender, BotCommandWithResponder, ReceiverLease,
};
use crate::command_validate::clamp_to_i32;
use crate::compound_ops::find_standable_neighbor;
use crate::error::BotError;
use crate::state::SharedState;
use crate::tool_select::{build_tool_alternatives, find_tool_in_inventory};
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

    /// Like [`Self::goto`], but bound by an explicit deadline instead of the
    /// configured command timeout.
    ///
    /// The default implementation delegates to [`Self::goto`] so mock
    /// implementations that do not model a real pathfinder stay untouched;
    /// [`RealBotClient`] overrides it so an explicit timeout (e.g. the fly
    /// timeout) actually reaches the pathfinder's wait-for-completion loop —
    /// before this method existed, `fly_timeout_secs` only widened the outer
    /// envelope while the inner `goto` was still bounded by
    /// `command_timeout_secs` (audit M-1).
    async fn goto_with_deadline(
        &self,
        pos: &BlockPos,
        _deadline: Duration,
    ) -> Result<(), BotError> {
        self.goto(pos).await
    }

    /// Stop the currently running pathfinder (if any).
    ///
    /// Used after a movement timeout so the bot does not keep walking toward
    /// a goal the caller has already given up on. Must be callable at any
    /// time, including when no pathfinding is active.
    fn stop_pathfinding(&self);

    /// Whether the bot is currently within the pathfinder's "arrived"
    /// radius of the most recent `goto` goal.
    ///
    /// Used by [`RealBotClient::goto`] as a 50ms fallback in case the
    /// tick handler's `notify_waiters()` is delayed or dropped — without
    /// it, a missed tick would force callers to wait the full
    /// `command_timeout_secs` before returning. Mock implementations
    /// return `true` once the pathfinder has been told to start.
    fn is_goto_target_reached(&self) -> bool;

    /// The bot's current precise position, read live from the client.
    ///
    /// Unlike the world snapshot (throttled to `snapshot_interval_ms`, or
    /// up to 5 s when the bot is idle), this is the position at the moment
    /// of the call. Used by `handle_act` to report a `self_info.position`
    /// that reflects where the bot actually ended up after a movement —
    /// the throttled snapshot can lag a just-completed move by one whole
    /// interval, which LLM clients misread as "did not arrive".
    ///
    /// Returns `None` when the client position component is unavailable
    /// (offline or before the first sync); callers fall back to the snapshot.
    fn position(&self) -> Option<[f64; 3]>;

    /// Perform a single jump.
    async fn jump(&self);

    /// Switch to a hotbar slot (0–8).
    fn switch_hotbar_slot(&self, slot: u8);

    /// Drop items from an inventory slot (0-35).
    fn drop_item(&self, slot: u8, count: u8);

    /// Swap the stack in `source_menu_slot` with the hotbar slot
    /// `target_hotbar_slot` (0-8) via a container swap-click.
    ///
    /// `source_menu_slot` is a *menu* slot index: for the player menu the
    /// main inventory is 9-35 and the hotbar is 36-44. Used by
    /// [`BotCommand::MoveItemToHotbar`] to bring an inventory item into the
    /// hotbar without any server-side command. No-op when a container window
    /// is open (the click would target the container menu instead).
    fn swap_hotbar(&self, source_menu_slot: u16, target_hotbar_slot: u8);

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

    /// Number of occupied slots in the player's 36-slot inventory, read
    /// live from the currently open menu.
    ///
    /// Unlike [`Self::inventory_entries`] (which only sees the player
    /// menu), this works while a container menu is open: every non-player
    /// menu carries the player inventory in its trailing slots
    /// (`Menu::player_slots_range`), kept in sync by the server's
    /// container-content packets. Used by the `take_from_container`
    /// inventory-full guard (audit F6-3).
    fn player_inventory_occupied_slots(&self) -> usize;
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

    /// Shared body of [`BotActions::goto`] and
    /// [`BotActions::goto_with_deadline`]: start the pathfinder toward `pos`
    /// and wait for completion bounded by `timeout_dur`.
    ///
    /// The wait/fallback loop is a free function so the 50ms fallback
    /// semantics can be unit-tested with a mock `BotActions` implementation
    /// (a real azalea `Client` cannot be constructed in tests).
    async fn goto_inner(&self, pos: &BlockPos, timeout_dur: Duration) -> Result<(), BotError> {
        let az_pos = azalea::BlockPos::new(pos.x, pos.y, pos.z);
        let goal = BlockPosGoal(az_pos);
        let timeout_secs = timeout_dur.as_secs();
        let notify = self.state.goto_notify();

        self.client.goto(goal).await;

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

    fn position(&self) -> Option<[f64; 3]> {
        self.client
            .get_component::<azalea::entity::Position>()
            .map(|pos| [pos.x, pos.y, pos.z])
    }

    async fn goto(&self, pos: &BlockPos) -> Result<(), BotError> {
        // Honour the user-configured command timeout — read it through the
        // sender so the value is in lock-step with the timeout
        // `BotCommandSender::send_command` itself uses.
        let timeout_dur = self.sender.timeout();
        self.goto_inner(pos, timeout_dur).await
    }

    async fn goto_with_deadline(&self, pos: &BlockPos, deadline: Duration) -> Result<(), BotError> {
        // M-1: same as `goto`, but the wait-for-completion loop is bounded by
        // the EXPLICIT deadline (e.g. the fly timeout) instead of the
        // configured command timeout — so long flights no longer give up at
        // `command_timeout_secs`.
        self.goto_inner(pos, deadline).await
    }

    fn stop_pathfinding(&self) {
        self.client.stop_pathfinding();
    }

    async fn jump(&self) {
        self.client.set_jumping(true);
        // A full Minecraft jump takes ~300ms from lift-off to landing; the
        // previous 100ms cut the jump short before the bot left the ground.
        sleep(Duration::from_millis(300)).await;
        self.client.set_jumping(false);
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

    fn swap_hotbar(&self, source_menu_slot: u16, target_hotbar_slot: u8) {
        use azalea_inventory::operations::SwapClick;

        // The container click is only valid against the player menu (window
        // id 0). While a container is open `get_inventory()` returns a handle
        // targeting the container window, so the swap would act on the wrong
        // menu — bail out and let the caller report the limitation.
        let inventory = self.client.get_inventory();
        if inventory.id() != 0 {
            warn!(
                source_menu_slot,
                target_hotbar_slot,
                "swap_hotbar skipped: a container window is open (id {}), \
                 swap would target the container menu",
                inventory.id()
            );
            return;
        }
        inventory.click(SwapClick {
            source_slot: source_menu_slot,
            target_slot: target_hotbar_slot,
        });
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
        let eid = clamp_to_i32(entity_id);
        let entity = self
            .client
            .ecs_entity_by_minecraft_entity(azalea::world::MinecraftEntityId(eid))
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
        // Read the player's 36 slots in canonical order (hotbar 0-8 first,
        // main inventory 9-35), via the shared helper. This also works while
        // a container menu is open — the trailing 36 player slots are read
        // through `Menu::player_slots_range()` — unlike the previous
        // `try_as_player()` fallback which returned an empty list.
        canonical_player_inventory(&self.client.menu())
    }

    fn player_inventory_occupied_slots(&self) -> usize {
        // Readable even while a container menu is open: every non-player
        // menu carries the player's 36 slots in its trailing positions
        // (`Menu::player_slots_range`), and the server keeps them in sync
        // via container-content packets. For the player menu itself the
        // range covers the same 36 inventory slots.
        let menu = self.client.menu();
        let slots = menu.slots();
        let range = menu.player_slots_range();
        slots[range]
            .iter()
            .filter(|stack| !stack.is_empty())
            .count()
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
// Player inventory slot canonicalization
// ═══════════════════════════════════════════════════════════════
//
// azalea's `Menu::Player.inventory` (and the trailing 36 player slots of
// every other menu) is laid out in *protocol* order: **main inventory first**
// (27 slots) then **hotbar last** (9 slots). The rest of the crate — the
// snapshot's `InventorySlot.slot_index`, `equip_tool`, `drop_item`,
// `MoveItemToHotbar`, and `select_tool_for_block` — assumes the
// canonical **hotbar-first** order (index 0..=8 = hotbar, 9..=35 = main
// inventory). These two helpers are the single place that reconciles the
// two orderings.

/// Map an azalea player-inventory slot index (0-35, main-inventory-first) to
/// the canonical index used across the crate (hotbar 0-8 first, then main
/// inventory 9-35).
///
/// azalea stores 27 main-inventory slots at indices 0..=26 and 9 hotbar
/// slots at 27..=35; canonical order is the reverse. The 27/9 split is
/// guaranteed by `Menu::hotbar_slots_range()` ("hotbar is always last 9
/// slots in the player's inventory").
pub(crate) fn canonical_inventory_slot(azalea_slot: usize) -> usize {
    if azalea_slot < 27 {
        azalea_slot + 9
    } else {
        azalea_slot - 27
    }
}

/// Read the player's 36 inventory slots in canonical order (hotbar 0-8 first,
/// then main inventory 9-35), regardless of whether a container menu is open.
///
/// Uses `Menu::player_slots_range()` — the trailing 36 player slots present
/// in **every** menu — rather than `try_as_player()`, so a chest/furnace
/// window no longer hides the inventory. Each slot is then re-ordered through
/// `canonical_inventory_slot` so hotbar items land at indices 0-8.
pub(crate) fn canonical_player_inventory(menu: &azalea_inventory::Menu) -> Vec<Option<ItemStack>> {
    let slots = menu.slots();
    let player_range = menu.player_slots_range();
    let mut out: Vec<Option<ItemStack>> = vec![None; 36];
    for (az_idx, stack) in slots[player_range].iter().enumerate() {
        if stack.is_empty() {
            continue;
        }
        out[canonical_inventory_slot(az_idx)] = Some(ItemStack {
            item_id: item_kind_to_id(stack.kind()),
            count: stack.count().clamp(0, 255) as u8,
        });
    }
    out
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

/// Fold a sub-operation [`BotResult`] into the `(action_result, reason)`
/// pair consumed by [`ActResult`].
///
/// A sub-result with `success: false` carries its message as the `reason`
/// so that `handle_act`'s wrapping success flag (derived from
/// `reason.is_none()`) honestly reports the failure — e.g. a SmartMove
/// blocked by an obstacle must not surface as a successful `act` (F2-6).
/// Sub-operations that only ever return `success: true` on `Ok` are
/// unaffected.
fn act_outcome(result: BotResult) -> (String, Option<String>) {
    if result.success {
        (result.message, None)
    } else {
        (result.message.clone(), Some(result.message))
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
            // Destructure before dispatch so the command is moved (not
            // cloned) into the handler and the responder stays usable after.
            let BotCommandWithResponder {
                command,
                respond_to,
            } = wrapped;
            debug!(command = ?command, "dispatching command");
            let result = self.dispatch(command).await;
            if respond_to.send(result).is_err() {
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
                    let BotCommandWithResponder {
                        command,
                        respond_to,
                    } = wrapped;
                    debug!(command = ?command, "dispatching command");
                    // Mark the executor busy for the duration of the command so
                    // query tools can tell that a `force` snapshot refresh may
                    // return pre-command state (the serial loop cannot process
                    // a refresh request while it is inside this command).
                    self.state.set_executor_busy(true);
                    let result = self.dispatch(command).await;
                    self.state.set_executor_busy(false);
                    if respond_to.send(result).is_err() {
                        warn!("command responder dropped — result lost");
                    }
                }
                None => break,
            }
        }

        self.state.set_executor_busy(false);
        trace!("command executor loop ended (channel closed)");
    }

    /// Dispatch a single command and record it in the run stats.
    ///
    /// Every command that reaches the executor (including compound-operation
    /// sub-commands) counts toward the UI's "Command Stats" panel — see
    /// [`RunStats`](crate::config::RunStats). Validation rejections and
    /// offline denials count as failures, because the panel reports
    /// "received commands", not "executed commands".
    pub(crate) async fn dispatch(&self, cmd: BotCommand) -> Result<BotResult, BotError> {
        let result = self.dispatch_inner(cmd).await;
        {
            // Scope the stats guard tightly: the atomics do not need the
            // guard held past this block.
            let stats = self.state.read_run_stats();
            stats.commands_processed.fetch_add(1, Ordering::Relaxed);
            if result.is_ok() {
                stats.commands_succeeded.fetch_add(1, Ordering::Relaxed);
            } else {
                stats.commands_failed.fetch_add(1, Ordering::Relaxed);
            }
        }
        result
    }

    /// Execute a single command without recording run stats.
    ///
    /// The actual dispatch logic lives here so [`dispatch`](Self::dispatch)
    /// can wrap it with the stats bookkeeping without changing the match.
    async fn dispatch_inner(&self, cmd: BotCommand) -> Result<BotResult, BotError> {
        // Defense-in-depth: validate parameter bounds for every command before
        // execution. MCP handlers validate too, but this central gate catches
        // any handler that misses a bound (container slot/count, walk distance)
        // and covers commands generated internally by compound operations.
        crate::command_validate::validate_command(&cmd)?;

        // Record command activity so the snapshot updater keeps its fast
        // rebuild interval while commands keep arriving (idle bots relax to
        // a slower interval).
        self.state.mark_command_activity();

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
            BotCommand::Teleport(pos) => self.handle_teleport(pos).await,

            // ── Block interaction ─────────────────────────────────
            BotCommand::BreakBlock(pos) => self.handle_break_block(pos),
            BotCommand::PlaceBlock(pos, block_type) => {
                self.handle_place_block(pos, block_type).await
            }
            BotCommand::UseItemOnBlock(pos, item_slot, effect_pos) => {
                self.handle_use_item_on_block(pos, item_slot, effect_pos)
                    .await
            }

            // ── Item / inventory ──────────────────────────────────
            BotCommand::SwitchHotbarSlot(slot) => self.handle_switch_hotbar_slot(slot),
            BotCommand::DropItem(slot, count) => self.handle_drop_item(slot, count).await,
            BotCommand::MoveItemToHotbar(hotbar_slot, item_id, count) => {
                self.handle_move_item_to_hotbar(hotbar_slot, item_id, count)
                    .await
            }
            BotCommand::UseItem => self.handle_use_item(),
            BotCommand::UseItemWithSlot(slot) => self.handle_use_item_with_slot(slot),
            BotCommand::EquipTool(tool) => self.handle_equip_tool(tool, None).await,
            BotCommand::EquipToolWithMaterial(tool, material) => {
                self.handle_equip_tool(tool, Some(material)).await
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
            BotCommand::AttackEntity(id) => self.handle_attack_entity(id).await,
            BotCommand::ShieldBlock(blocking) => self.handle_shield_block(blocking),

            // ── Chat / command ────────────────────────────────────
            BotCommand::SendChat(msg) => self.handle_send_chat(msg),
            BotCommand::ExecuteCommand(cmd) => self.handle_execute_command(cmd).await,
            BotCommand::SetGameMode(mode) => self.handle_set_game_mode(mode),

            // ── Queries ───────────────────────────────────────────
            BotCommand::QueryInventory => self.handle_query_inventory(),

            // ── v2 foundation: extended capabilities ──────────────
            BotCommand::SmartMove(target) => self.handle_smart_move(target).await,
            BotCommand::FlyTo(target) => self.handle_fly_to(target).await,
            BotCommand::CollectItems(radius) => self.handle_collect_items(radius).await,
            BotCommand::Act(action, perception) => self.handle_act(action, perception).await,
        }
    }

    // ── Movement handlers ────────────────────────────────────────

    async fn handle_move_to(&self, pos: BlockPos) -> Result<BotResult, BotError> {
        trace!(?pos, "MoveTo");
        match self.goto_with_margin(pos).await {
            GotoOutcome::Completed(Ok(())) => {
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
            GotoOutcome::Completed(Err(e)) => Err(e),
            // The envelope timeout is about to fire — return a structured
            // partial result (current position + distance covered) instead of
            // letting `send_command` produce a bare `CommandTimeout`.
            GotoOutcome::TimedOut {
                start,
                current,
                timeout_secs,
            } => Ok(movement_timeout_result(
                "MoveTo",
                pos,
                start,
                current,
                timeout_secs,
            )),
        }
    }

    /// Run [`BotActions::goto`] with a margin short of the command envelope
    /// timeout so a long movement returns a *structured* "timed out" result
    /// (with current position and distance moved) instead of a bare
    /// `BotError::CommandTimeout` from `send_command`.
    ///
    /// On timeout the pathfinder is stopped so the bot does not keep walking
    /// toward a goal the caller has already given up on.
    async fn goto_with_margin(&self, target: BlockPos) -> GotoOutcome {
        self.goto_with_margin_with_timeout(target, None).await
    }

    /// [`Self::goto_with_margin`] with an explicit timeout override.
    ///
    /// `timeout_secs` replaces the configured `command_timeout_secs` (used
    /// by `fly_to`, whose long flights need their own `fly_timeout_secs`
    /// — see [`crate::config::AppConfig::fly_timeout_secs`]).
    async fn goto_with_margin_with_timeout(
        &self,
        target: BlockPos,
        timeout_secs: Option<u64>,
    ) -> GotoOutcome {
        // Read the configured timeout from shared state — the same value
        // `BotCommandSender::send_command`'s envelope timeout uses (the
        // per-command fly override matches `timeout_for`).
        let timeout_dur = Duration::from_secs(
            timeout_secs.unwrap_or_else(|| self.state.read_config().command_timeout_secs),
        );
        let timeout_secs = timeout_dur.as_secs();
        // Leave a small margin for the executor to reply before the envelope
        // timeout in `BotCommandSender::send_command` abandons the response.
        let goto_window = timeout_dur.saturating_sub(MOVEMENT_REPLY_MARGIN);
        let start = self.state.read_snapshot().self_player.position;
        // Route through `goto_with_deadline` so the explicit timeout (e.g.
        // the fly timeout) reaches the pathfinder's wait-for-completion loop
        // — not just the outer envelope (audit M-1).
        match tokio::time::timeout(
            goto_window,
            self.bot.goto_with_deadline(&target, timeout_dur),
        )
        .await
        {
            Ok(result) => GotoOutcome::Completed(result),
            Err(_) => {
                self.bot.stop_pathfinding();
                let current = self.state.read_snapshot().self_player.position;
                GotoOutcome::TimedOut {
                    start,
                    current,
                    timeout_secs,
                }
            }
        }
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
                let origin = self.state.read_snapshot().self_player.position;
                // Clamp to i32 range so a malicious or malformed `distance`
                // (u32 > i32::MAX) doesn't silently wrap to a negative
                // offset, which would make the bot walk in the opposite
                // direction.  Saturating add guards the coordinate
                // arithmetic against overflow from extreme inputs.
                let d = clamp_to_i32(distance);
                let target = BlockPos::new(
                    origin.x.saturating_add(dx.saturating_mul(d)),
                    origin.y.saturating_add(dy.saturating_mul(d)),
                    origin.z.saturating_add(dz.saturating_mul(d)),
                );
                // M-5: route through the margin wrapper so the executor
                // replies BEFORE the `send_command` envelope fires. A bare
                // `goto` raced the envelope (same timeout value, no margin),
                // so a successful movement that hit the deadline was reported
                // as Err(CommandTimeout). On the margin deadline the handler
                // returns the structured partial result instead.
                match self.goto_with_margin(target).await {
                    GotoOutcome::TimedOut {
                        start,
                        current,
                        timeout_secs,
                    } => {
                        return Ok(movement_timeout_result(
                            "WalkDirection",
                            target,
                            start,
                            current,
                            timeout_secs,
                        ));
                    }
                    GotoOutcome::Completed(Err(e)) => return Err(e),
                    GotoOutcome::Completed(Ok(())) => {}
                }

                if !self.state.is_online() {
                    return Err(BotError::Offline("disconnected during movement".into()));
                }

                // R-12: report where the bot actually ended up. Prefer the
                // zero-wait live position component — the throttled snapshot
                // can lag a just-finished move by one interval (5 s when
                // idle) — and fall back to the snapshot when unavailable.
                let live_position = self.bot.position();
                let end_pos = live_position
                    .map(|[x, y, z]| {
                        BlockPos::new(x.floor() as i32, y.floor() as i32, z.floor() as i32)
                    })
                    .unwrap_or_else(|| self.state.read_snapshot().self_player.position);

                Ok(BotResult {
                    success: true,
                    message: format!("Walking {:?} for {} blocks", dir, distance),
                    data: Some(serde_json::json!({
                        "position": [end_pos.x, end_pos.y, end_pos.z],
                    })),
                })
            }
            None => {
                // Up/Down: azalea's pathfinder has no vertical-only goal.
                // `direction_to_vector` returns `None` for these. This is a
                // caller input error, not an internal failure — report
                // `InvalidParams` so the MCP client sees a proper
                // INVALID_PARAMS response with actionable guidance.
                Err(BotError::InvalidParams(format!(
                    "direction {dir:?} is not supported for distance-based movement; \
                     use north/south/east/west or a diagonal"
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

    async fn handle_teleport(&self, pos: BlockPos) -> Result<BotResult, BotError> {
        trace!(?pos, "Teleport");
        self.teleport_via_command(pos).await
    }

    /// Teleport via the server-authoritative `/tp` command.
    ///
    /// The previous implementation mutated the player's local ECS `Position`
    /// component and reported success, but the server re-syncs the
    /// authoritative position every tick, so the bot never actually moved —
    /// the tool reported a fake success. `/tp` is the only reliable path, and
    /// it needs operator permissions (creative mode alone is not enough on a
    /// vanilla server). The command is verified against the chat feedback
    /// exactly like `handle_execute_command`: a rejection (no OP) surfaces
    /// `CommandRejected` instead of a fake success.
    async fn teleport_via_command(&self, pos: BlockPos) -> Result<BotResult, BotError> {
        trace!(?pos, "TeleportViaCommand");
        let command = format!("/tp {} {} {}", pos.x, pos.y, pos.z);
        let baseline_seq = self.state.chat_cursor();
        self.bot.chat(&command);

        // Give the server a short window to reply with rejection feedback.
        sleep(Duration::from_millis(COMMAND_FEEDBACK_WAIT_MS)).await;

        if let Some(rejection) = self.rejection_feedback_after(baseline_seq) {
            return Err(BotError::CommandRejected {
                command,
                feedback: rejection,
            });
        }

        // No rejection — the tp was accepted; attach the newest system
        // feedback (e.g. "Teleported X to ...") so the client sees the
        // server-confirmed destination.
        let feedback = self.server_feedback_after(baseline_seq);
        let message = match &feedback {
            Some(fb) => format!("Teleported to {} (server: {fb})", pos),
            None => format!("Teleported to {}", pos),
        };
        Ok(BotResult {
            success: true,
            message,
            data: feedback.map(|fb| serde_json::json!({ "feedback": fb })),
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

    /// Place a block that will occupy `pos`, verifying the placement landed.
    ///
    /// `pos` is the position the placed block is expected to OCCUPY. Because
    /// azalea's `block_interact` fabricates an Up-face hit (AGENTS.md R-9),
    /// the executor right-clicks the cell below (`pos − Up`) so the block
    /// lands at `pos`. The handler:
    ///
    /// 1. Rejects `pos.y` outside `-63..=320` — the click target (`pos − Up`)
    ///    would be out of the world below y=-64 (the MCP layer validates
    ///    too; this is defense-in-depth for internal dispatchers).
    /// 2. Pre-checks via the snapshot: the effect cell `pos` must be empty
    ///    (air, or absent — production snapshots filter air entries, so
    ///    absent == empty) and the click target `pos − Up` must be a loaded
    ///    block (else `ChunkNotLoaded`). An occupied effect cell is an
    ///    `InvalidParams` ("already occupied").
    /// 3. Auto-approaches when the bot is farther than the 4.5-block
    ///    survival reach — the server silently drops out-of-range
    ///    interactions, the historical fake-success bug.
    /// 4. Switches the hotbar slot, right-clicks `pos − Up`, then VERIFIES
    ///    via [`wait_for_block_present`] that a block actually appeared at
    ///    `pos`. A verification timeout returns an honest `success:false`
    ///    naming the block and the cell — never an unconditional success.
    async fn handle_place_block(
        &self,
        pos: BlockPos,
        block_type: String,
    ) -> Result<BotResult, BotError> {
        trace!(?pos, %block_type, "PlaceBlock");
        // Defense-in-depth: the placed block occupies `pos`, so the click
        // target is `pos − Up`. A `pos.y` of -64 would put the click target
        // at y=-65, outside the world. `validate_command`'s bounds are
        // -64..=320, so this narrower gate is reachable via dispatch for
        // y=-64 — reject honestly (MCP layer validates too, contract H-2).
        if pos.y < -63 || pos.y > 320 {
            return Err(BotError::InvalidParams(format!(
                "cannot place a block at {pos}: y must be in -63..=320 \
                 (the click target below it would be out of the world)"
            )));
        }

        // The MCP layer encodes the hotbar slot as "slot:N" in the block_type
        // field (see tools_block::handle_place_block). Select that slot before
        // right-clicking so the correct block is placed.
        //
        // A "slot:" prefix whose payload is not a valid hotbar index (parse
        // failure or > 8) is a malformed internal encoding. Fail honestly with
        // `InvalidParams` instead of warn-and-continue-with-held-item, which
        // would place the WRONG block and still report success (F2-5).
        let slot_switch = if let Some(slot_str) = block_type.strip_prefix("slot:") {
            Some(match slot_str.parse::<u8>() {
                Ok(s) if s <= 8 => s,
                _ => {
                    return Err(BotError::InvalidParams(format!(
                        "invalid internal slot encoding: {block_type}"
                    )));
                }
            })
        } else {
            None
        };
        // Strip the internal "slot:N" prefix (if any) from the result message
        // so the LLM sees a clean block type name rather than an opaque
        // hotbar index like "Placed slot:3 at ...".
        let display_type = block_type.strip_prefix("slot:").unwrap_or(&block_type);

        // Pre-checks: the effect cell `pos` must be empty and the click
        // target `pos − Up` must be a loaded block in the snapshot.
        let click_target = BlockPos::new(pos.x, pos.y - 1, pos.z);
        let snapshot = self.state.read_snapshot();
        if !snapshot.block_index.contains_key(&click_target) {
            return Err(BotError::ChunkNotLoaded(click_target));
        }
        if let Some(&idx) = snapshot.block_index.get(&pos) {
            let occupant = snapshot.blocks[idx].block_type.clone();
            // `"air"` is the mock/defensive fallback spelling; production
            // snapshots never store air entries (audit L-4), so an entry
            // that is present and non-air genuinely occupies the cell.
            if !occupant.eq_ignore_ascii_case("air") {
                return Err(BotError::InvalidParams(format!(
                    "cannot place a block at {pos}: the cell is already occupied by {occupant}"
                )));
            }
        }

        // Auto-approach when out of interaction range. Minecraft's survival
        // reach is 4.5 blocks; the server silently drops interactions beyond
        // it (the historical fake-success bug).
        const INTERACT_REACH: f64 = 4.5;
        let player_pos = snapshot.self_player.position;
        if distance_between(player_pos, pos) > INTERACT_REACH {
            let fresh = self.state.read_snapshot();
            let stand = match find_standable_neighbor(&fresh, click_target) {
                Some(s) => s,
                None => {
                    return Err(BotError::Internal(format!(
                        "no standable position adjacent to placement target {pos}"
                    )));
                }
            };
            // Best-effort approach: a movement failure falls through to the
            // (now honest) result below rather than aborting early.
            let _ = self.goto_with_margin(stand).await;
        }

        // Switch to the hotbar slot and right-click the cell below `pos` so
        // the block lands at `pos` (azalea's fabricated Up-face hit).
        if let Some(slot) = slot_switch {
            self.bot.switch_hotbar_slot(slot);
        }
        self.bot.block_interact(&click_target);

        // Server-side confirmation: poll the snapshot until a block appears
        // at `pos`. A timeout is an honest failure — never a fake success.
        let budget = Duration::from_millis(self.state.read_config().snapshot_interval_ms + 250);
        if wait_for_block_present(&self.state, pos, budget).await {
            return Ok(BotResult {
                success: true,
                message: format!("Placed {display_type} at {pos}"),
                data: Some(serde_json::json!({
                    "verified": true,
                    "position": pos,
                })),
            });
        }
        warn!(
            ?pos,
            ?click_target,
            %display_type,
            "place_block: no block appeared at the effect cell"
        );
        Ok(BotResult {
            success: false,
            message: format!(
                "place_block: {display_type} did not appear at {pos} within {} ms (clicked {click_target}); \
                 the interaction was likely rejected by the server",
                budget.as_millis()
            ),
            data: Some(serde_json::json!({
                "verified": false,
                "reason": "placement_not_observed",
                "position": pos,
            })),
        })
    }

    /// Use an item on a block, with optional placement verification.
    ///
    /// `pos` is the block actually right-clicked; `effect_pos` (when `Some`)
    /// is the position where the item's effect is EXPECTED to land (e.g. the
    /// cell a water bucket should fill). With `effect_pos` set the handler:
    ///
    /// 1. Pre-checks `pos` is in the world snapshot and `effect_pos` is
    ///    currently air/empty — an occupied target yields an explicit
    ///    `InvalidParams` ("cannot place into an occupied cell") instead of a
    ///    fake success.
    /// 2. Auto-approaches `pos` when the bot is out of interaction range
    ///    (4.5 blocks, survival reach) — the server silently rejects
    ///    out-of-range interactions, which previously surfaced as
    ///    "success with no world change".
    /// 3. After the click, polls the snapshot until `effect_pos` turns
    ///    non-air (server-side confirmation). A timeout returns an explicit
    ///    `success: false` result with the held item named, instead of the
    ///    old fire-and-forget "success".
    ///
    /// Items without a placement effect (tools, food, interactables like
    /// chests) skip verification and keep the legacy "Used X" report.
    async fn handle_use_item_on_block(
        &self,
        pos: BlockPos,
        item_slot: Option<u8>,
        effect_pos: Option<BlockPos>,
    ) -> Result<BotResult, BotError> {
        trace!(?pos, ?item_slot, ?effect_pos, "UseItemOnBlock");
        // If a hotbar slot was specified, select it before interacting so the
        // correct item is used. Mirrors `handle_switch_hotbar_slot`'s range
        // check (the MCP layer also validates, but defend in depth).
        if let Some(slot) = item_slot
            && slot > 8
        {
            // Defense-in-depth only — dispatch's central `validate_command`
            // gate rejects this first. Carry the honest variant anyway: an
            // out-of-range slot is a caller input error, not internal.
            return Err(BotError::InvalidParams(format!(
                "item_slot {slot} out of hotbar range (0-8)"
            )));
        }

        let snapshot = self.state.read_snapshot();

        // Identify the item that will be used (for the confirmation gate and
        // the result message).
        let used_slot = item_slot.unwrap_or(snapshot.self_player.held_item_slot);
        let used_item = snapshot
            .self_player
            .inventory
            .iter()
            .find(|entry| entry.slot_index == used_slot)
            .map(|entry| entry.item_id.as_str())
            .unwrap_or("empty")
            .to_string();
        // M-6: whether the used item is expected to place a block/fluid. The
        // effect-cell occupancy pre-check AND the auto-approach only make
        // sense for placement items — for flint-and-steel, a cauldron-filling
        // bucket, etc. the "effect cell" is meaningless and a ceiling above
        // the target caused a false `InvalidParams` (audit M-6).
        let is_placement = item_has_placement_effect(&used_item);

        // Placement verification path (fluid buckets / placeable blocks).
        if let Some(effect) = effect_pos {
            // (1) Pre-check the interaction target exists in the snapshot
            // and the effect cell is currently empty (air or unknown).
            // Gated on the item being a placement item: non-placement items
            // skip the occupancy gate entirely.
            if is_placement {
                if !snapshot.block_index.contains_key(&pos) {
                    return Err(BotError::ChunkNotLoaded(pos));
                }
                let effect_occupied = snapshot
                    .block_index
                    .get(&effect)
                    .map(|&idx| !snapshot.blocks[idx].block_type.eq_ignore_ascii_case("air"))
                    .unwrap_or(false);
                if effect_occupied {
                    let occupant = snapshot
                        .block_index
                        .get(&effect)
                        .map(|&idx| snapshot.blocks[idx].block_type.clone())
                        .unwrap_or_default();
                    return Err(BotError::InvalidParams(format!(
                        "cannot use item at {}: target cell {} is already occupied by {occupant} (a fluid bucket/block lands in the cell the used face opens into)",
                        pos, effect
                    )));
                }

                // (2) Auto-approach when out of interaction range. Minecraft's
                // survival reach is 4.5 blocks; the server silently drops
                // interactions beyond it (the historical fake-success bug).
                const INTERACT_REACH: f64 = 4.5;
                let player_pos = snapshot.self_player.position;
                if distance_between(player_pos, pos) > INTERACT_REACH {
                    let fresh = self.state.read_snapshot();
                    let stand = match find_standable_neighbor(&fresh, pos) {
                        Some(s) => s,
                        None => {
                            return Err(BotError::Internal(format!(
                                "no standable position adjacent to interaction target {pos}"
                            )));
                        }
                    };
                    // Best-effort approach: a movement failure falls through to
                    // the (now honest) result below rather than aborting early.
                    let _ = self.goto_with_margin(stand).await;
                }
            }
        }

        // (3) Switch slot and right-click.
        if let Some(slot) = item_slot {
            self.bot.switch_hotbar_slot(slot);
        }
        self.bot.block_interact(&pos);

        // (4) Server-side confirmation: poll the snapshot until the effect
        // cell turns non-air. Only placement items are verified; everything
        // else keeps the legacy report.
        if let Some(effect) = effect_pos
            && is_placement
        {
            let budget = Duration::from_millis(self.state.read_config().snapshot_interval_ms + 250);
            if wait_for_block_present(&self.state, effect, budget).await {
                return Ok(BotResult {
                    success: true,
                    message: format!(
                        "Used {used_item} on block at {} (slot {used_slot}); confirmed at {}",
                        pos, effect
                    ),
                    data: Some(serde_json::json!({
                        "verified": true,
                        "effect_position": effect,
                        "item": used_item,
                    })),
                });
            }
            // Honest failure: the click was sent but the world never changed.
            warn!(
                ?pos,
                ?effect,
                %used_item,
                "use_item_on_block: no effect observed at target cell"
            );
            // Fluid buckets get a targeted explanation: azalea 0.15.1's
            // `block_interact` fabricates the hit result (block centre,
            // fixed Up face — see the interact plugin), which vanilla
            // rejects for bucket `UseItemOn` while accepting block
            // placements and flint-and-steel on the same path. There is no
            // upstream API to send a real raycast hit, so the honest answer
            // is a specific "unsupported" error plus a working alternative
            // rather than a generic "interaction was likely rejected".
            if is_fluid_bucket_item(&used_item) {
                return Ok(BotResult {
                    success: false,
                    message: format!(
                        "use_item_on_block: {used_item} placement failed at {effect} (clicked {pos}) — azalea 0.15.1's block interaction layer cannot place fluid buckets (the fabricated hit is rejected by the server); use execute_command '/setblock {} {} {} {}' or '/fill' instead",
                        effect.x,
                        effect.y,
                        effect.z,
                        used_item
                            .strip_prefix("minecraft:")
                            .unwrap_or(&used_item)
                            .trim_end_matches("_bucket"),
                    ),
                    data: Some(serde_json::json!({
                        "verified": false,
                        "reason": "bucket_placement_unsupported",
                        "effect_position": effect,
                        "item": used_item,
                        "alternatives": [
                            format!("/setblock {} {} {} <fluid>", effect.x, effect.y, effect.z),
                            "/fill <x1> <y1> <z1> <x2> <y2> <z2> water",
                        ],
                    })),
                });
            }
            return Ok(BotResult {
                success: false,
                message: format!(
                    "use_item_on_block: no effect observed at {effect} within {} ms (used {used_item} on {pos}); the interaction was likely rejected by the server",
                    budget.as_millis()
                ),
                data: Some(serde_json::json!({
                    "verified": false,
                    "effect_position": effect,
                    "item": used_item,
                })),
            });
        }

        Ok(BotResult {
            success: true,
            message: format!("Used {used_item} on block at {} (slot {used_slot})", pos),
            data: None,
        })
    }

    // ── Item / inventory handlers ────────────────────────────────

    fn handle_switch_hotbar_slot(&self, slot: u8) -> Result<BotResult, BotError> {
        trace!(slot, "SwitchHotbarSlot");
        if slot > 8 {
            // Defense-in-depth only — dispatch's central `validate_command`
            // gate rejects this first. Carry the honest variant anyway.
            return Err(BotError::InvalidParams(format!(
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

    /// Drop items from a slot and verify the drop actually landed.
    ///
    /// The previous implementation returned success unconditionally, so a
    /// drop that was silently rejected by the server (e.g. the slot was
    /// already empty, or a container window was open and the click targeted
    /// the wrong menu) still reported "Dropped N item(s)". We now read the
    /// inventory before and after the click and report the truth.
    async fn handle_drop_item(&self, slot: u8, count: u8) -> Result<BotResult, BotError> {
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

        // Snapshot the affected slot before the click. A None entry means the
        // slot was already empty — the click would throw from an empty slot,
        // which the server may reject silently.
        let before = self.bot.inventory_entries();
        let before_count = before
            .get(slot as usize)
            .and_then(|opt| opt.as_ref())
            .map(|stack| stack.count)
            .unwrap_or(0);

        self.bot.drop_item(slot, count);

        // Give the server a moment to apply the click and send the
        // container-content packets back.
        sleep(Duration::from_millis(250)).await;
        let after = self.bot.inventory_entries();

        let removed = before_count.saturating_sub(
            after
                .get(slot as usize)
                .and_then(|opt| opt.as_ref())
                .map(|stack| stack.count)
                .unwrap_or(0),
        );

        // Defensive branch (now unreachable via the real client, which always
        // returns 36 slots through 'canonical_player_inventory'): a non-player
        // menu open during the drop makes the click target the wrong window.
        // Report honestly instead of faking success if the read ever comes
        // back empty.
        if after.is_empty() && !before.is_empty() {
            return Ok(BotResult {
                success: true,
                message: format!(
                    "Dropped {} item(s) from slot {} (container window open; drop may not apply until the container is closed)",
                    count, slot
                ),
                data: Some(serde_json::json!({"verified": false, "container_window_open": true})),
            });
        }

        if removed >= count {
            Ok(BotResult {
                success: true,
                message: format!("Dropped {} item(s) from slot {}", count, slot),
                data: Some(serde_json::json!({"verified": true, "removed": removed})),
            })
        } else if before_count == 0 {
            // Slot was empty before the drop — the click had nothing to throw.
            Ok(BotResult {
                success: false,
                message: format!(
                    "Drop did not change inventory slot {slot}: the slot is already empty"
                ),
                data: Some(serde_json::json!({"verified": false, "removed": 0})),
            })
        } else {
            // The server rejected the click or the slot count did not drop.
            Ok(BotResult {
                success: false,
                message: format!(
                    "Drop did not change inventory slot {slot}: server rejected the click or the window did not match (removed {removed} of {count})"
                ),
                data: Some(serde_json::json!({"verified": false, "removed": removed})),
            })
        }
    }

    /// Move an inventory stack into a hotbar slot via a container swap-click.
    ///
    /// Finds the first inventory slot holding at least `count` of `item_id`,
    /// then swaps it with hotbar slot `hotbar_slot` (0-8). This is the
    /// in-game "press the hotbar number while clicking a slot" operation and
    /// needs no server-side command — it only moves items that already exist
    /// in the inventory, so it cannot conjure items (that still requires an
    /// `/give`-style command).
    async fn handle_move_item_to_hotbar(
        &self,
        hotbar_slot: u8,
        item_id: String,
        count: u8,
    ) -> Result<BotResult, BotError> {
        trace!(hotbar_slot, %item_id, count, "MoveItemToHotbar");
        let entries = self.bot.inventory_entries();

        // Locate the first slot with a matching item id and enough count.
        let source_idx = entries.iter().position(
            |opt| matches!(opt, Some(stack) if stack.item_id == item_id && stack.count >= count),
        );

        let Some(source_idx) = source_idx else {
            return Err(BotError::InvalidParams(format!(
                "item_id '{item_id}' not found in inventory (need at least {count} in one slot)"
            )));
        };

        // Logical inventory index → player-menu slot: hotbar 0-8 maps to menu
        // 36-44, main inventory 9-35 maps to menu 9-35 (identity).
        let source_menu_slot: u16 = if source_idx <= 8 {
            36 + source_idx as u16
        } else {
            source_idx as u16
        };

        self.bot.swap_hotbar(source_menu_slot, hotbar_slot);

        // Wait for the server to apply the swap and send container-content
        // packets back, then verify the hotbar slot now holds the item.
        sleep(Duration::from_millis(250)).await;
        let verified = matches!(
            self.bot.inventory_entries().get(hotbar_slot as usize),
            Some(Some(stack)) if stack.item_id == item_id && stack.count >= count
        );

        Ok(BotResult {
            success: verified,
            message: format!(
                "Moved {}x {} into hotbar slot {} (source inventory slot {source_idx}){}",
                count,
                item_id,
                hotbar_slot,
                if verified {
                    ""
                } else {
                    " — unverified: a container window may be open"
                }
            ),
            data: Some(serde_json::json!({
                "verified": verified,
                "source_slot": source_idx,
                "source_menu_slot": source_menu_slot,
                "hotbar_slot": hotbar_slot,
            })),
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

    async fn handle_equip_tool(
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
            Some((_material, slot)) => {
                // Tool is in the main inventory (slot 9-35). Auto-move it into
                // the first free hotbar slot via a container swap-click, then
                // switch to it — instead of erroring out. This is the
                // "degrade equip_tool" path: the tool exists, so equip it
                // rather than asking the caller to rearrange the hotbar.
                let item_id = entries
                    .get(slot as usize)
                    .and_then(|opt| opt.as_ref())
                    .map(|stack| stack.item_id.clone())
                    .ok_or_else(|| {
                        BotError::Internal(format!(
                            "tool slot {slot} disappeared from inventory during equip"
                        ))
                    })?;
                // Prefer an empty hotbar slot; when the hotbar is full, fall
                // back to slot 0 (the swap trades places, so the displaced
                // hotbar item lands in the tool's old main-inventory slot).
                let target = (0..=8u8)
                    .find(|&i| entries[i as usize].is_none())
                    .unwrap_or(0);
                let moved = self.handle_move_item_to_hotbar(target, item_id, 1).await?;
                if !moved.success {
                    return Ok(moved);
                }
                self.bot.switch_hotbar_slot(target);
                Ok(BotResult {
                    success: true,
                    message: format!(
                        "Equipped {tool:?} from main inventory (moved to hotbar slot {target})"
                    ),
                    data: None,
                })
            }
            None => Err(BotError::ToolNotFound {
                tool_type: tool,
                material,
                alternatives: build_tool_alternatives(tool, required_level),
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
        // F6-3: fail fast if the player inventory has no free slots. This
        // prevents shift-clicking a container stack that would be dropped
        // or lost. The check reads the inventory LIVE from the currently
        // open menu (container menus carry the player inventory in their
        // trailing slots): the previous snapshot-based check could never
        // fire, because the snapshot inventory is always empty while a
        // container menu is open.
        if self.bot.player_inventory_occupied_slots() >= 36 {
            return Err(BotError::InventoryFull);
        }
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

    /// Attack an entity by its Minecraft entity ID.
    ///
    /// Entities move, so a stale snapshot position can make a direct attack
    /// fail with a range error. When the target is farther than
    /// `MAX_ATTACK_REACH` away, the bot paths to the entity's last known
    /// position first, then re-reads the snapshot and attacks only when the
    /// entity is now within reach. If it moved on (or despawned) during the
    /// approach, the caller gets the honest `TooFar` / `InvalidParams`
    /// error and should re-run `get_nearby_entities` → `move_to` → attack.
    async fn handle_attack_entity(&self, entity_id: u32) -> Result<BotResult, BotError> {
        trace!(entity_id, "AttackEntity");
        // Fail fast if the target entity is outside a reasonable attack reach.
        // The snapshot may be slightly stale, so the threshold is generous.
        const MAX_ATTACK_REACH: f64 = 6.0;
        let snapshot = self.state.read_snapshot();
        let Some(entity) = snapshot.entities.iter().find(|e| e.id == entity_id) else {
            // Mirrors the MCP layer's existence check: attacking an entity we
            // cannot see in the snapshot is a parameter error, not a bot
            // failure — the ID may be stale or fabricated.
            drop(snapshot);
            return Err(BotError::InvalidParams(format!(
                "Entity with ID {entity_id} not found in current world snapshot"
            )));
        };
        let entity_pos = entity.position;
        let distance = distance_between(entity_pos, snapshot.self_player.position);
        let mut approached = false;
        if distance > MAX_ATTACK_REACH {
            // Moving-target fix: approach the entity's last known position
            // before attacking. A movement timeout also falls through — the
            // fresh re-check below decides the outcome either way.
            drop(snapshot);
            let _ = self.goto_with_margin(entity_pos).await;
            approached = true;
        }
        // (On the direct-attack path the snapshot guard simply falls out of
        // scope here — no long await between its last use and this point.)

        // Fresh read after the (potential) approach: the entity may have
        // moved while the bot walked, and the snapshot refreshes on its own
        // tick. Attack only when the target is within reach *now*.
        let fresh = self.state.read_snapshot();
        let Some(current_entity) = fresh.entities.iter().find(|e| e.id == entity_id) else {
            return Err(BotError::InvalidParams(format!(
                "Entity with ID {entity_id} no longer in the world snapshot"
            )));
        };
        let fresh_distance = distance_between(current_entity.position, fresh.self_player.position);
        if fresh_distance > MAX_ATTACK_REACH {
            return Err(BotError::TooFar {
                target: current_entity.position,
                current: fresh.self_player.position,
                max_distance: MAX_ATTACK_REACH,
            });
        }
        drop(fresh);

        self.bot.attack_entity(entity_id)?;
        // Report the bot's position: the auto-approach above may have moved
        // the bot, and the caller must see that drift in the result.
        let final_pos = self.state.read_snapshot().self_player.position;
        Ok(BotResult {
            success: true,
            message: if approached {
                format!(
                    "Attacked entity {entity_id} after approaching (distance {fresh_distance:.1})"
                )
            } else {
                format!("Attacked entity {entity_id}")
            },
            data: Some(serde_json::json!({"position": final_pos})),
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

    /// Execute a Minecraft command and verify the server did not reject it.
    ///
    /// The server reports command failures (e.g. `Incorrect argument for
    /// command ...`, `Unknown command`) as a chat/system message rather than
    /// an error packet. We record the chat baseline, send the command, wait a
    /// short window for feedback, and diff the new system messages: any one
    /// matching a known rejection pattern is returned as
    /// [`BotError::CommandRejected`] so the MCP client learns the truth
    /// instead of seeing a fake success (previously `execute_command` always
    /// returned "Executed command: ..."). When no rejection is detected the
    /// command is reported as executed, with the newest system feedback
    /// attached to the message when available.
    async fn handle_execute_command(&self, cmd: String) -> Result<BotResult, BotError> {
        trace!(%cmd, "ExecuteCommand");
        // The MCP layer (tools_chat::handle_execute_command) already
        // normalises the leading `/`, so `cmd` is passed straight to chat.
        // Re-prepending here would produce `//command`, which Minecraft
        // treats as a normal chat message rather than a command.
        // The baseline is a monotonic chat cursor, NOT a list length: the
        // chat deque is capped, so when it is full a length baseline is
        // always the cap and an index-based diff skips every new message
        // (rejection detection silently stopped working in real sessions).
        let baseline_seq = self.state.chat_cursor();
        self.bot.chat(&cmd);

        // Give the server a short window to reply with rejection feedback.
        sleep(Duration::from_millis(COMMAND_FEEDBACK_WAIT_MS)).await;

        // Scan every System message that arrived in the window — Minecraft
        // reports a rejection as TWO messages (the error title plus the
        // command echo with a `<--[HERE]` marker), so checking only the
        // newest one would miss the rejection whenever the echo lands last.
        if let Some(rejection) = self.rejection_feedback_after(baseline_seq) {
            return Err(BotError::CommandRejected {
                command: cmd.clone(),
                feedback: rejection,
            });
        }

        // No rejection detected — report success, attaching the newest system
        // feedback (if any) so clients can still see e.g. the result of the
        // command ("Teleported X to ...", "Seed: [...]").
        let feedback = self.server_feedback_after(baseline_seq);
        let message = match &feedback {
            Some(fb) => format!("Executed command: {} (server: {fb})", cmd),
            None => format!("Executed command: {}", cmd),
        };
        Ok(BotResult {
            success: true,
            message,
            data: feedback.map(|fb| serde_json::json!({ "feedback": fb })),
        })
    }

    /// Collect system chat messages that arrived after `baseline_seq`
    /// (strictly after the pre-command baseline), returning the newest one.
    fn server_feedback_after(&self, baseline_seq: u64) -> Option<String> {
        self.state
            .chat_messages_since(baseline_seq)
            .iter()
            .filter(|(_, sender, _)| sender.eq_ignore_ascii_case("System"))
            .map(|(_, _, message)| message.clone())
            .next_back()
    }

    /// Return the newest System message after `baseline_seq` that matches a
    /// known command-rejection pattern, if any.
    ///
    /// The rejection scan checks **every** message in the window, not just
    /// the newest: Minecraft's command errors arrive as two System messages
    /// (the error title carrying the keyword, e.g. "Unknown or incomplete
    /// command. See below for error", followed by the command echo with a
    /// `<--[HERE]` marker). `server_feedback_after`'s newest-only selection
    /// is correct for success feedback but misses rejections whose echo
    /// lands last.
    fn rejection_feedback_after(&self, baseline_seq: u64) -> Option<String> {
        self.state
            .chat_messages_since(baseline_seq)
            .iter()
            .filter(|(_, sender, message)| {
                sender.eq_ignore_ascii_case("System") && is_command_rejection(message)
            })
            .map(|(_, _, message)| message.clone())
            .next_back()
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

    // ── v2 foundation handlers ───────────────────────────────────

    /// Smart movement with auto-jump. Azalea's pathfinder already handles
    /// 1-block auto-jumps, so this delegates to [`BotActions::goto`] and
    /// inspects the result to report whether the target was reached or an
    /// obstacle blocked progress.
    async fn handle_smart_move(&self, target: BlockPos) -> Result<BotResult, BotError> {
        trace!(?target, "SmartMove");

        // Transient pathfinding misreads (e.g. the grass block underfoot
        // momentarily reported as an obstacle) usually resolve on a second
        // attempt. Retry exactly once before declaring an obstacle; a
        // TimedOut already spent the full movement window, so it is never
        // retried (retrying would double the latency for no benefit).
        let mut retried = false;
        loop {
            match self.goto_with_margin(target).await {
                GotoOutcome::TimedOut {
                    start,
                    current,
                    timeout_secs,
                } => {
                    return Ok(movement_timeout_result(
                        "SmartMove",
                        target,
                        start,
                        current,
                        timeout_secs,
                    ));
                }
                GotoOutcome::Completed(goto_result) => {
                    // M-4: prefer the bot's LIVE position (zero-wait) over
                    // the throttled snapshot for the reached check. The
                    // snapshot can lag a long move by a whole idle interval
                    // (5 s — `effective_snapshot_interval_ms` relaxes to 5 s
                    // when no commands arrive), which made a physically-
                    // completed move look unreached and smart_move falsely
                    // reported an "obstacle". Falls back to the snapshot
                    // when the position component is unavailable (offline /
                    // before the first sync). Mirrors `handle_walk_direction`.
                    let live_position = self.bot.position();
                    let current_pos = live_position
                        .map(|[x, y, z]| {
                            BlockPos::new(x.floor() as i32, y.floor() as i32, z.floor() as i32)
                        })
                        .unwrap_or_else(|| self.state.read_snapshot().self_player.position);

                    // Reached when the bot ends within one block of the target
                    // on every axis (the same margin the previous logic used).
                    let reached = (current_pos.x - target.x).abs() <= 1
                        && (current_pos.y - target.y).abs() <= 1
                        && (current_pos.z - target.z).abs() <= 1;

                    if !reached && !retried {
                        // First attempt stopped short (Ok-but-unreached or a
                        // pathfinder error). Give the world a beat and retry
                        // once before declaring an obstacle.
                        retried = true;
                        tokio::time::sleep(SMART_MOVE_RETRY_DELAY).await;
                        continue;
                    }

                    let reason = if reached { "reached" } else { "obstacle" };
                    let obstacle = if reached {
                        None
                    } else {
                        // Look for a solid block directly ahead (between
                        // current and target) to report as the obstacle.
                        find_obstacle_block(&self.state.read_snapshot(), current_pos, target)
                    };
                    let message = match (&goto_result, reached) {
                        (Err(e), false) => format!("SmartMove to {target} blocked: {e}"),
                        _ => format!("SmartMove to {target}: {reason}"),
                    };

                    // F2-6: `success` must reflect whether the target was
                    // actually reached. Previously blocked branches reported
                    // `success: true` — dishonest.
                    return Ok(BotResult {
                        success: reached,
                        message,
                        data: Some(serde_json::json!({
                            "reached": reached,
                            "reason": reason,
                            "position": [current_pos.x, current_pos.y, current_pos.z],
                            "obstacle": obstacle.map(obstacle_to_json),
                            "retried": retried,
                        })),
                    });
                }
            }
        }
    }

    /// Creative-mode flight to a position. If the bot is not in creative mode,
    /// returns `not_creative`.
    ///
    /// azalea's pathfinder is ground-based (walk/jump/fall) and cannot change
    /// the player's Y beyond a 1-block jump, so the flight is split in two:
    /// (1) path horizontally to the target XZ at the current Y via
    /// [`BotActions::goto`], then (2) complete the vertical delta (and any
    /// residual horizontal offset) with a server-authoritative `/tp`
    /// ([`teleport_via_command`]). Creative flight has no fall damage, so the
    /// teleport is safe; the previous local-ECS position mutation was silently
    /// reverted by the server every tick.
    async fn handle_fly_to(&self, target: BlockPos) -> Result<BotResult, BotError> {
        trace!(?target, "FlyTo");
        let snapshot = self.state.read_snapshot();
        let gamemode = snapshot.self_player.gamemode;
        let current_pos = snapshot.self_player.position;

        if gamemode != GameMode::Creative {
            // Act(Fly) reaches this handler without the MCP layer's creative
            // gate, so enforce it here too. Returning `success: true` with
            // `reached: false` was dishonest — the bot cannot fly, so the
            // action failed.
            return Err(BotError::PermissionDenied(
                "FlyTo requires Creative mode".into(),
            ));
        }

        // Horizontal leg: path to the target XZ at the current Y. Keeping Y
        // unchanged makes the goal reachable by azalea's ground pathfinder.
        let horizontal_target = BlockPos::new(target.x, current_pos.y, target.z);
        // Long flights need a longer window than ordinary commands: use the
        // dedicated fly timeout (default 60 s).
        let fly_timeout = self.state.read_config().fly_timeout_secs;
        match self
            .goto_with_margin_with_timeout(horizontal_target, Some(fly_timeout))
            .await
        {
            GotoOutcome::TimedOut {
                start,
                current,
                timeout_secs,
            } => Ok(movement_timeout_result(
                "FlyTo",
                target,
                start,
                current,
                timeout_secs,
            )),
            GotoOutcome::Completed(goto_result) => match goto_result {
                // Horizontal pathfinding failed (obstacle): report it
                // honestly and do NOT teleport past the blockage.
                Err(e) => {
                    let pos = self.state.read_snapshot().self_player.position;
                    Ok(BotResult {
                        success: false,
                        message: format!("FlyTo {target} blocked: {e}"),
                        data: Some(serde_json::json!({
                            "reached": false,
                            "reason": "obstacle",
                            "position": [pos.x, pos.y, pos.z],
                        })),
                    })
                }
                // Horizontal leg reached: complete the vertical delta (and
                // any residual horizontal offset) with a server-authoritative
                // /tp — the local-ECS position mutation was silently reverted
                // by the server every tick, so the bot never actually landed
                // at the target.
                Ok(()) => {
                    let tp_result = self.teleport_via_command(target).await?;
                    let mut data = serde_json::json!({
                        "reached": true,
                        "reason": "reached",
                        "position": [target.x, target.y, target.z],
                    });
                    if let Some(fb) = tp_result.data {
                        data.as_object_mut()
                            .expect("data is an object")
                            .insert("feedback".into(), fb);
                    }
                    Ok(BotResult {
                        success: true,
                        message: format!("FlyTo {}: reached", target),
                        data: Some(data),
                    })
                }
            },
        }
    }

    /// Collect dropped item entities within `radius` blocks of the player.
    ///
    /// Walks to each item entity in turn; auto-pickup happens when the bot
    /// gets close. Returns the count of item entities visited.
    async fn handle_collect_items(&self, radius: u32) -> Result<BotResult, BotError> {
        trace!(radius, "CollectItems");
        let snapshot = self.state.read_snapshot();
        let player_pos = snapshot.self_player.position;
        let r = clamp_to_i32(radius);

        // Filter for dropped-item entities within radius. azalea reports
        // dropped items as `item` (and some versions as `item_entity`).
        // `is_collectible_item_entity` matches only those two —
        // `item_frame` and `item_display` are not pickup-able.
        let item_targets: Vec<BlockPos> = snapshot
            .entities
            .iter()
            .filter(|e| is_collectible_item_entity(&e.entity_type))
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

        // M-5: total movement budget = the command envelope minus the reply
        // margin. Without the margin a raw `goto` races the envelope (same
        // timeout value) and a success at the deadline is reported as
        // CommandTimeout; a multi-target loop can exceed the envelope
        // entirely and drop the visited count.
        let total_budget = Duration::from_secs(self.state.read_config().command_timeout_secs)
            .saturating_sub(MOVEMENT_REPLY_MARGIN);
        let mut remaining_budget = total_budget;
        let mut visited: u32 = 0;
        let mut timed_out = false;
        let item_count = item_targets.len();

        for (i, target) in item_targets.iter().enumerate() {
            // Split the remaining budget across the targets still to visit,
            // with a 2-second floor so a single short hop never gets a
            // sub-second window.
            let remaining_targets = item_count - i;
            let per_target = remaining_budget
                .checked_div(remaining_targets as u32)
                .map(|d| d.max(Duration::from_secs(2)))
                .unwrap_or(Duration::from_secs(2));
            match self
                .goto_with_margin_with_timeout(*target, Some(per_target.as_secs().max(1)))
                .await
            {
                GotoOutcome::TimedOut { .. } => {
                    // Budget exhausted — stop so the reply still lands before
                    // the envelope fires, and report the partial visit count
                    // honestly below (never an Err, never a fake full count).
                    timed_out = true;
                    break;
                }
                GotoOutcome::Completed(Ok(())) => {
                    // Brief pause for the server to process pickup.
                    sleep(Duration::from_millis(200)).await;
                    visited += 1;
                }
                GotoOutcome::Completed(Err(_)) => {
                    // A pathfinding failure is not a budget problem — move on
                    // to the next target (mirrors the old loop's is_ok()
                    // skip).
                }
            }
            remaining_budget = remaining_budget.saturating_sub(per_target);
        }

        // Report the bot's END position: collect_items walks toward every
        // target, so callers must be able to detect the drift without a
        // separate get_self_info round-trip.
        let final_pos = self.state.read_snapshot().self_player.position;
        let skipped = item_count as u32 - visited;
        let message = if timed_out {
            format!(
                "Visited {visited} of {item_count} item drop location(s); remaining {skipped} skipped — movement budget exhausted"
            )
        } else {
            format!("Visited {visited} item drop location(s); auto-pickup expected on proximity")
        };
        Ok(BotResult {
            success: !timed_out,
            message,
            data: Some(serde_json::json!({
                "visited": visited,
                "position": final_pos,
                "timed_out": timed_out,
            })),
        })
    }

    /// Unified Act tool — dispatches the inner [`ActAction`] to the
    /// appropriate handler, then wraps the result in an [`ActResult`]
    /// enriched with nearby blocks/entities and self info from the snapshot.
    ///
    /// `perception` is the per-call radius (blocks, Chebyshev, 0..=32)
    /// bounding the nearby blocks/entities payload; `None` falls back to the
    /// configured `block_perception_radius`. Callers that only need the
    /// action outcome pass `Some(0)` to strip the nearby context entirely.
    async fn handle_act(
        &self,
        action: ActAction,
        perception: Option<u32>,
    ) -> Result<BotResult, BotError> {
        trace!(?action, "Act");
        let (action_result, reason): (String, Option<String>) = match action {
            ActAction::Move { target } => match self.handle_move_to(target).await {
                Ok(r) => act_outcome(r),
                Err(e) => ("failed".into(), Some(e.to_string())),
            },
            ActAction::SmartMove { target } => match self.handle_smart_move(target).await {
                Ok(r) => act_outcome(r),
                Err(e) => ("failed".into(), Some(e.to_string())),
            },
            ActAction::Fly { target } => match self.handle_fly_to(target).await {
                Ok(r) => act_outcome(r),
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
                    Ok(r) => act_outcome(r),
                    Err(e) => ("failed".into(), Some(e.to_string())),
                }
            }
            ActAction::Attack { entity_id } => match self.handle_attack_entity(entity_id).await {
                Ok(r) => act_outcome(r),
                Err(e) => ("failed".into(), Some(e.to_string())),
            },
            ActAction::CollectItems { radius } => match self.handle_collect_items(radius).await {
                Ok(r) => act_outcome(r),
                Err(e) => ("failed".into(), Some(e.to_string())),
            },
        };

        // Build the enriched result from the current snapshot, but prefer the
        // bot's LIVE position for the nearby-filter centre and `self_info`:
        // the snapshot is throttled (up to `snapshot_interval_ms`, or 5 s
        // when idle), so right after a movement it can still report the
        // pre-move position and an LLM client would misread a successful
        // move as "did not arrive". `BotActions::position` is a zero-wait
        // live read; when it is unavailable (offline / before first sync)
        // the snapshot value is kept.
        let snapshot = self.state.read_snapshot();
        let live_position = self.bot.position();
        let player_pos = live_position
            .map(|[x, y, z]| BlockPos::new(x.floor() as i32, y.floor() as i32, z.floor() as i32))
            .unwrap_or(snapshot.self_player.position);
        // L-15: build the ActResult's self_info from a CLONE of the small
        // `SelfPlayer` with the live-position overrides applied — never
        // `Arc::make_mut(&mut snapshot)`, which deep-clones the whole
        // snapshot (blocks + block_index, potentially huge) on every act
        // call because the Arc is shared by readers. The override stays
        // local to this result; the shared snapshot is untouched.
        let mut self_info = snapshot.self_player.clone();
        if let Some(live) = live_position {
            self_info.position = player_pos;
            self_info.position_precise = Some(live);
        }
        // Per-call override (0..=32, validated upstream) or the configured
        // default. Small radii keep the ActResult payload tiny — the default
        // radius-32 result serialises to over 1 MB of nearby-block JSON.
        let perception_radius: i32 = perception
            .map(|r| r as i32)
            .unwrap_or_else(|| self.state.read_config().block_perception_radius as i32);

        // radius=0 must return NO nearby context at all (the tool contract
        // says "0 returns no nearby context at all"). The `<= 0` filter below
        // would otherwise keep entities/blocks sharing the player's own cell.
        let (nearby_blocks, nearby_entities): (Vec<_>, Vec<_>) = if perception_radius == 0 {
            (Vec::new(), Vec::new())
        } else {
            let blocks: Vec<_> = snapshot
                .blocks
                .iter()
                .filter(|b| {
                    (b.position.x - player_pos.x).abs() <= perception_radius
                        && (b.position.y - player_pos.y).abs() <= perception_radius
                        && (b.position.z - player_pos.z).abs() <= perception_radius
                })
                .cloned()
                .collect();
            let entities: Vec<_> = snapshot
                .entities
                .iter()
                .filter(|e| {
                    (e.position.x - player_pos.x).abs() <= perception_radius
                        && (e.position.y - player_pos.y).abs() <= perception_radius
                        && (e.position.z - player_pos.z).abs() <= perception_radius
                })
                .cloned()
                .collect();
            (blocks, entities)
        };

        let act_result = ActResult {
            action_result,
            reason,
            nearby_blocks,
            nearby_entities,
            self_info,
        };

        Ok(BotResult {
            success: act_result.reason.is_none(),
            message: "Act completed".into(),
            data: Some(serde_json::to_value(&act_result).unwrap_or_default()),
        })
    }
}

// ═══════════════════════════════════════════════════════════════
// Free-function helpers for v2 handlers
// ═══════════════════════════════════════════════════════════════

/// Euclidean distance between two block positions (used by the attack reach
/// checks). Pure f64 arithmetic — no allocation, no snapshot access.
fn distance_between(a: BlockPos, b: BlockPos) -> f64 {
    let dx = (a.x - b.x) as f64;
    let dy = (a.y - b.y) as f64;
    let dz = (a.z - b.z) as f64;
    (dx * dx + dy * dy + dz * dz).sqrt()
}

/// Whether using this item is expected to place a block/fluid in the world
/// (thus needs server-side confirmation after the click).
///
/// Fluid buckets (water/lava/powder snow) place a fluid block; milk does
/// not. Placeable blocks are detected via the block-to-tool table, which
/// carries an entry for every mined/placed block.
fn item_has_placement_effect(item_id: &str) -> bool {
    if is_fluid_bucket_item(item_id) {
        return true;
    }
    crate::block_data::BLOCK_TO_TOOL_TYPE.contains_key(item_id)
}

/// Whether the item id is a fluid bucket (water/lava/powder snow), excluding
/// milk which is a consumable rather than a placement item.
fn is_fluid_bucket_item(item_id: &str) -> bool {
    item_id.ends_with("_bucket") && !item_id.contains("milk")
}

/// Whether an entity type string is a dropped-item entity that can be picked
/// up by walking over it.
///
/// azalea reports dropped items as `item` (and some versions as
/// `item_entity`). `item_frame` and `item_display` are block-like/display
/// entities and must NOT be treated as drops.
fn is_collectible_item_entity(entity_type: &str) -> bool {
    let etype = entity_type.to_lowercase();
    etype == "item" || etype == "item_entity"
}

/// How long the executor waits for server feedback after sending a
/// `/execute`-style command before deciding whether it was rejected.
///
/// The server replies to command failures (and most successes) over chat
/// within a few hundred milliseconds; waiting longer would delay every
/// `execute_command` call without improving detection.
const COMMAND_FEEDBACK_WAIT_MS: u64 = 400;

/// Margin (in seconds) subtracted from the command timeout when wrapping a
/// movement `goto`. The movement must report back *before* the
/// `BotCommandSender::send_command` envelope timeout, otherwise the MCP
/// client sees a bare `CommandTimeout` instead of the structured partial
/// result (`position`, `distance`, ...) this executor builds.
const MOVEMENT_REPLY_MARGIN: Duration = Duration::from_secs(1);

/// Pause between the first and the single retry of a smart_move whose first
/// attempt stopped short — lets the server settle a transient misread (e.g.
/// the block underfoot reported as an obstacle) before trying again.
const SMART_MOVE_RETRY_DELAY: Duration = Duration::from_millis(300);

/// Outcome of a [`CommandExecutor::goto_with_margin`] run.
enum GotoOutcome {
    /// The inner `goto` finished before the margin deadline.
    Completed(Result<(), BotError>),
    /// The margin deadline elapsed before `goto` returned — the pathfinder
    /// was stopped and a structured partial result should be reported.
    TimedOut {
        /// Player position when the movement started.
        start: BlockPos,
        /// Player position when the deadline elapsed (may be unchanged).
        current: BlockPos,
        /// The configured command timeout in seconds.
        timeout_secs: u64,
    },
}

/// Build the structured "movement timed out" [`BotResult`] returned when a
/// `goto` exceeds the command timeout. Carries the current position and the
/// distance actually covered so the MCP client can decide whether to retry,
/// continue, or cancel — instead of a bare `CommandTimeout` error with no
/// context (S-fix #3).
fn movement_timeout_result(
    op: &str,
    target: BlockPos,
    start: BlockPos,
    current: BlockPos,
    timeout_secs: u64,
) -> BotResult {
    let dx = (current.x - start.x) as f64;
    let dy = (current.y - start.y) as f64;
    let dz = (current.z - start.z) as f64;
    let distance = (dx * dx + dy * dy + dz * dz).sqrt();
    BotResult {
        success: false,
        message: format!(
            "{op} to {target} timed out after {timeout_secs}s (moved {distance:.1} blocks; bot stopped)"
        ),
        data: Some(serde_json::json!({
            "reason": "timeout",
            "timeout_secs": timeout_secs,
            "position": [current.x, current.y, current.z],
            "start": [start.x, start.y, start.z],
            "target": [target.x, target.y, target.z],
            "distance_moved": distance,
            "distance_xyz": [dx.abs(), dy.abs(), dz.abs()],
        })),
    }
}

/// Serialize an [`ObstacleInfo`] into the smart-move result payload.
fn obstacle_to_json(obstacle: ObstacleInfo) -> serde_json::Value {
    serde_json::json!({
        "block_type": obstacle.block_type,
        "x": obstacle.position.x,
        "y": obstacle.position.y,
        "z": obstacle.position.z,
    })
}

/// A solid block reported as the obstacle that blocked movement.
///
/// Unlike the previous `Option<String>` (block type only), carrying the
/// position lets the MCP client show *where* the bot is stuck.
struct ObstacleInfo {
    /// Snapshot block type (e.g. `"stone"`).
    block_type: String,
    /// World position of the blocking block.
    position: BlockPos,
}

/// Case-insensitive substring match against the server's known
/// command-rejection messages (vanilla and common plugin servers).
fn is_command_rejection(feedback: &str) -> bool {
    const PATTERNS: &[&str] = &[
        "incorrect argument for command",
        "unknown or incomplete command",
        "unknown command",
        "unknown item",
        "no such item",
        "you do not have permission to use this command",
        "you are not allowed to use this command",
        "cannot execute",
    ];
    let lower = feedback.to_lowercase();
    PATTERNS.iter().any(|p| lower.contains(p))
}

/// Cap on the interpolated line scan's step count.
///
/// Validated coordinates span ±30,000,000 (WORLD_BORDER), so a cross-world
/// SmartMove diagnostic could otherwise walk up to 60,000,000 cells × 3 Y
/// layers on the serial executor's critical path. The scan starts at
/// `current` and walks toward `target`, so the obstacle that actually
/// stopped the bot is always near the start of the line — cells beyond the
/// cap add no diagnostic value. Lines longer than the cap fall through to
/// the 3×3×3 neighbourhood scan when the capped prefix is clear.
const MAX_OBSTACLE_SCAN_STEPS: i64 = 10_000;

/// Find the first solid block between `current` and `target` to report as
/// the obstacle that blocked movement.
///
/// Scans the interpolated XZ line (proportional in both axes so the scan
/// follows the real path instead of a 45° diagonal) across three Y layers
/// (current Y, Y+1, Y-1 — an obstacle may sit at a different height than the
/// bot's feet, e.g. a ledge). If the line is clear but the bot still stopped
/// short, falls back to scanning the 3×3×3 neighbourhood around `current`.
/// Returns `None` only if no solid block is found anywhere.
///
/// All interpolation runs in `i64`: coordinates are validated to ±30,000,000,
/// so `total_dx * i` reaches ~3.6e14 for long lines — far beyond `i32::MAX`
/// (~2.1e9, already exceeded at `i ≈ 36` of a maximum-length line). The old
/// all-`i32` arithmetic panicked debug builds and wrapped to garbage
/// coordinates in release builds (S-3).
fn find_obstacle_block(
    snapshot: &crate::types::WorldSnapshot,
    current: BlockPos,
    target: BlockPos,
) -> Option<ObstacleInfo> {
    let start_x = i64::from(current.x);
    let start_z = i64::from(current.z);
    let total_dx = i64::from(target.x) - start_x;
    let total_dz = i64::from(target.z) - start_z;
    // True line length in cells; the interpolation ALWAYS divides by this so
    // each step advances exactly one cell along the line. The cap only limits
    // how many cells are visited from current toward target.
    let total_steps = total_dx.abs().max(total_dz.abs());
    let steps = total_steps.min(MAX_OBSTACLE_SCAN_STEPS);

    // Obstacles usually sit at the bot's feet level (y), at head level (y+1),
    // or a step below (y-1). Checking all three covers ledges and low walls.
    for dy in [0i32, 1, -1] {
        let y = current.y + dy;
        if steps > 0 {
            for i in 1..=steps {
                // The interpolated point always lies between current and
                // target (both validated i32 coordinates), so the narrowing
                // conversion cannot fail; the fallback keeps the scan
                // defensive regardless.
                let x = i32::try_from(start_x + total_dx * i / total_steps).unwrap_or(current.x);
                let z = i32::try_from(start_z + total_dz * i / total_steps).unwrap_or(current.z);
                let pos = BlockPos::new(x, y, z);
                if let Some(obstacle) = solid_block_at(snapshot, pos) {
                    return Some(obstacle);
                }
            }
        }
    }

    // Line scan found nothing (or the bot never left `current`) — look at the
    // surrounding blocks so the client still learns what surrounds the bot.
    for dx in -1..=1 {
        for dy in -1..=1 {
            for dz in -1..=1 {
                if dx == 0 && dy == 0 && dz == 0 {
                    continue;
                }
                let pos = BlockPos::new(current.x + dx, current.y + dy, current.z + dz);
                if let Some(obstacle) = solid_block_at(snapshot, pos) {
                    return Some(obstacle);
                }
            }
        }
    }

    None
}

/// Return an [`ObstacleInfo`] for `pos` if the snapshot records a solid
/// (non-air, non-empty) block there.
fn solid_block_at(snapshot: &crate::types::WorldSnapshot, pos: BlockPos) -> Option<ObstacleInfo> {
    let idx = snapshot.block_index.get(&pos)?;
    let block = &snapshot.blocks[*idx];
    if block.block_type.is_empty() || block.block_type == "air" {
        return None;
    }
    Some(ObstacleInfo {
        block_type: block.block_type.clone(),
        position: pos,
    })
}

// ═══════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::{BotCommandSender, create_command_channel};
    use crate::config::AppConfig;
    use crate::types::{
        BlockEntry, EntityEntry, InventorySlot, MaterialTier, SelfPlayer, ToolType,
    };
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    // ═══════════════════════════════════════════════════════════════
    // MockBotClient
    // ═══════════════════════════════════════════════════════════════

    /// Tracks which methods were called and with what arguments.
    #[derive(Debug)]
    struct MockCallLog {
        goto_calls: Mutex<Vec<BlockPos>>,
        /// Deadline durations passed to `goto_with_deadline` (M-1). Records
        /// the per-call deadline so tests can assert the fly timeout actually
        /// reaches the pathfinder's wait loop instead of the command timeout.
        goto_with_deadline_calls: Mutex<Vec<(BlockPos, Duration)>>,
        goto_succeeds: AtomicBool,
        /// Number of upcoming goto attempts that must fail (transient-error
        /// simulation). Decremented per goto call.
        goto_failures_remaining: AtomicUsize,
        /// When set, a successful goto also moves the shared snapshot's
        /// player position (default off — existing tests assert log-only
        /// movement). Enables tests of handlers that re-read the snapshot
        /// after movement (smart_move retry, attack auto-approach).
        goto_updates_snapshot: AtomicBool,
        snapshot_for_goto: Mutex<Option<Arc<crate::state::SharedState>>>,
        /// When `true`, `is_goto_target_reached` always returns `false`,
        /// forcing `RealBotClient::goto`'s 50ms fallback loop to keep
        /// spinning until `command_timeout_secs` elapses. Default is
        /// `false` so the existing tests still see "arrived on first
        /// 50ms tick".
        goto_target_unreached: AtomicBool,
        /// When `true`, `goto` never completes (simulates an unreachable
        /// target / stalled pathfinder). Used to exercise the
        /// `goto_with_margin` timeout path. Default `false`.
        goto_hangs: AtomicBool,
        /// When `> 0`, the Nth goto call never completes (like `goto_hangs`,
        /// but only from a specific call onward). Lets tests simulate a
        /// budget timeout mid-way through a multi-target loop (M-5). Default
        /// `0` (disabled).
        goto_hangs_after_n: AtomicUsize,
        jump_calls: AtomicUsize,
        hotbar_switch_calls: Mutex<Vec<u8>>,
        drop_item_calls: Mutex<Vec<(u8, u8)>>,
        use_item_calls: AtomicUsize,
        chat_calls: Mutex<Vec<String>>,
        attack_calls: Mutex<Vec<u32>>,
        attack_succeeds: AtomicBool,
        crouch_calls: Mutex<Vec<bool>>,
        mine_calls: Mutex<Vec<BlockPos>>,
        interact_calls: Mutex<Vec<BlockPos>>,
        /// When `true`, `block_interact` simulates a successful placement by
        /// adding a "stone" block at the clicked cell **+ Up** — mirroring
        /// azalea's fabricated Up-face hit (AGENTS.md R-9): right-clicking
        /// `p` places the block at `p+Up`. `handle_place_block` relies on
        /// exactly that convention (it clicks `pos − Up` to place at `pos`).
        /// Default `false` — existing tests assert log-only interactions.
        interact_places_block: AtomicBool,
        /// SharedState mutated by `block_interact` when `interact_places_block`
        /// is set (bound via [`MockBotClient::bind_state`], same as goto).
        snapshot_for_interact: Mutex<Option<Arc<crate::state::SharedState>>>,
        container_open_calls: Mutex<Vec<BlockPos>>,
        inventory_calls: AtomicUsize,
        inventory: Mutex<Vec<Option<ItemStack>>>,
        /// Value returned by `player_inventory_occupied_slots` (F6-3).
        /// Default 0 (inventory has free slots).
        player_inventory_occupied: AtomicUsize,
        position: Mutex<BlockPos>,
        /// Precise position reported by `BotActions::position`. `None`
        /// (default) mirrors a client whose position component is missing,
        /// so handlers fall back to the snapshot — existing tests keep their
        /// behaviour. Tests that exercise the live-position override set
        /// this explicitly.
        live_position: Mutex<Option<[f64; 3]>>,
        stop_pathfinding_calls: AtomicUsize,
        swap_hotbar_calls: Mutex<Vec<(u16, u8)>>,
    }

    impl MockCallLog {
        fn new() -> Self {
            Self {
                goto_calls: Mutex::new(Vec::new()),
                goto_with_deadline_calls: Mutex::new(Vec::new()),
                goto_succeeds: AtomicBool::new(true),
                goto_failures_remaining: AtomicUsize::new(0),
                goto_updates_snapshot: AtomicBool::new(false),
                snapshot_for_goto: Mutex::new(None),
                goto_target_unreached: AtomicBool::new(false),
                goto_hangs: AtomicBool::new(false),
                goto_hangs_after_n: AtomicUsize::new(0),
                jump_calls: AtomicUsize::new(0),
                hotbar_switch_calls: Mutex::new(Vec::new()),
                drop_item_calls: Mutex::new(Vec::new()),
                use_item_calls: AtomicUsize::new(0),
                chat_calls: Mutex::new(Vec::new()),
                attack_calls: Mutex::new(Vec::new()),
                attack_succeeds: AtomicBool::new(true),
                crouch_calls: Mutex::new(Vec::new()),
                mine_calls: Mutex::new(Vec::new()),
                interact_calls: Mutex::new(Vec::new()),
                interact_places_block: AtomicBool::new(false),
                snapshot_for_interact: Mutex::new(None),
                container_open_calls: Mutex::new(Vec::new()),
                inventory_calls: AtomicUsize::new(0),
                inventory: Mutex::new(Vec::new()),
                player_inventory_occupied: AtomicUsize::new(0),
                position: Mutex::new(BlockPos::new(0, 64, 0)),
                live_position: Mutex::new(None),
                stop_pathfinding_calls: AtomicUsize::new(0),
                swap_hotbar_calls: Mutex::new(Vec::new()),
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

        /// Make successful goto calls also update the shared snapshot's
        /// player position (see `MockCallLog::goto_updates_snapshot`), and
        /// bind the state so `block_interact` can place blocks when
        /// `interact_places_block` is enabled.
        fn bind_state(&self, state: Arc<crate::state::SharedState>) {
            *self.log.snapshot_for_goto.lock().unwrap() = Some(state.clone());
            *self.log.snapshot_for_interact.lock().unwrap() = Some(state);
            self.log.goto_updates_snapshot.store(true, Ordering::SeqCst);
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

        fn position(&self) -> Option<[f64; 3]> {
            *self.log.live_position.lock().unwrap()
        }

        async fn goto(&self, pos: &BlockPos) -> Result<(), BotError> {
            self.log.goto_calls.lock().unwrap().push(*pos);
            // Hang-on-Nth-call simulation: when `goto_hangs_after_n` is set
            // to a positive value, every goto from that call onward never
            // completes (mirrors `goto_hangs`). Lets a multi-target loop hit
            // its per-target budget timeout mid-way (M-5).
            let hang_at = self.log.goto_hangs_after_n.load(Ordering::SeqCst);
            let call_idx = self.log.goto_calls.lock().unwrap().len();
            if self.log.goto_hangs.load(Ordering::SeqCst) || (hang_at > 0 && call_idx >= hang_at) {
                // Simulate an unreachable target: the pathfinder never
                // completes. The outer `goto_with_margin` timeout (or the
                // command envelope) is what releases the caller.
                loop {
                    tokio::task::yield_now().await;
                }
            }
            // Transient-failure simulation: consume one remaining failure and
            // report a pathfinding error before consulting goto_succeeds.
            if self.log.goto_failures_remaining.load(Ordering::SeqCst) > 0 {
                self.log
                    .goto_failures_remaining
                    .fetch_sub(1, Ordering::SeqCst);
                return Err(BotError::PathfindingFailed {
                    target: BlockPos {
                        x: pos.x,
                        y: pos.y,
                        z: pos.z,
                    },
                    reason: "mock transient pathfinding failure".into(),
                });
            }
            if self.log.goto_succeeds.load(Ordering::SeqCst) {
                *self.log.position.lock().unwrap() = *pos;
                if self.log.goto_updates_snapshot.load(Ordering::SeqCst)
                    && let Some(state) = self.log.snapshot_for_goto.lock().unwrap().clone()
                {
                    // Mirror the real client's tick-driven snapshot: a
                    // completed goto lands the new position in the shared
                    // snapshot so post-movement re-checks see it.
                    state.modify_snapshot(|snap| snap.self_player.position = *pos);
                }
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

        async fn goto_with_deadline(
            &self,
            pos: &BlockPos,
            deadline: Duration,
        ) -> Result<(), BotError> {
            // M-1: record the deadline the handler passes through, then
            // delegate to the plain `goto` behaviour (mock has no real
            // pathfinder, so the deadline itself does not bound anything).
            self.log
                .goto_with_deadline_calls
                .lock()
                .unwrap()
                .push((*pos, deadline));
            self.goto(pos).await
        }

        fn switch_hotbar_slot(&self, slot: u8) {
            self.log.hotbar_switch_calls.lock().unwrap().push(slot);
        }

        fn drop_item(&self, slot: u8, count: u8) {
            self.log.drop_item_calls.lock().unwrap().push((slot, count));
            // Simulate the server applying the throw-click so drop
            // verification sees a reduced stack (mirrors a real drop).
            let mut inv = self.log.inventory.lock().unwrap();
            if let Some(Some(stack)) = inv.get_mut(slot as usize) {
                stack.count = stack.count.saturating_sub(count);
                if stack.count == 0 {
                    inv[slot as usize] = None;
                }
            }
        }

        fn stop_pathfinding(&self) {
            self.log
                .stop_pathfinding_calls
                .fetch_add(1, Ordering::SeqCst);
        }

        fn swap_hotbar(&self, source_menu_slot: u16, target_hotbar_slot: u8) {
            self.log
                .swap_hotbar_calls
                .lock()
                .unwrap()
                .push((source_menu_slot, target_hotbar_slot));
            // Simulate the server-side swap so `handle_move_item_to_hotbar`'s
            // post-swap verification sees the item in the hotbar slot. Player
            // menu layout: menu 36-44 = logical hotbar 0-8, menu 9-35 =
            // logical 9-35 (identity).
            let source_idx = if source_menu_slot >= 36 {
                (source_menu_slot - 36) as usize
            } else {
                source_menu_slot as usize
            };
            let mut inv = self.log.inventory.lock().unwrap();
            if source_idx < inv.len() && (target_hotbar_slot as usize) < inv.len() {
                inv.swap(source_idx, target_hotbar_slot as usize);
            }
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
            // H-2 placement simulation: mirror azalea's fabricated Up-face
            // hit — right-clicking `p` places the block at `p + Up`, exactly
            // what `handle_place_block` depends on (it clicks `pos − Up` so
            // the placed block occupies `pos`). The placed entry is inserted
            // into BOTH `blocks` and `block_index` so the post-place
            // verification (`wait_for_block_present` → `block_index.get`)
            // can find it.
            if self.log.interact_places_block.load(Ordering::SeqCst)
                && let Some(state) = self.log.snapshot_for_interact.lock().unwrap().clone()
            {
                let placed_at = BlockPos::new(pos.x, pos.y + 1, pos.z);
                let mut snap = (*state.read_snapshot()).clone();
                let exists = snap.blocks.iter().any(|b| b.position == placed_at);
                if exists {
                    for b in snap.blocks.iter_mut() {
                        if b.position == placed_at {
                            b.block_type = "stone".into();
                        }
                    }
                } else {
                    let new_idx = snap.blocks.len();
                    snap.blocks.push(BlockEntry {
                        position: placed_at,
                        block_type: "stone".into(),
                        block_state: None,
                    });
                    snap.block_index.insert(placed_at, new_idx);
                }
                state.update_snapshot(snap);
            }
        }

        async fn open_container(&self, pos: &BlockPos) -> Result<(), BotError> {
            self.log.container_open_calls.lock().unwrap().push(*pos);
            Ok(())
        }

        fn inventory_entries(&self) -> Vec<Option<ItemStack>> {
            self.log.inventory_calls.fetch_add(1, Ordering::SeqCst);
            self.log.inventory.lock().unwrap().clone()
        }

        fn player_inventory_occupied_slots(&self) -> usize {
            self.log.player_inventory_occupied.load(Ordering::SeqCst)
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

    /// Like `make_executor`, but the mock's successful goto also moves the
    /// shared snapshot's player position — needed by handlers that re-read
    /// the snapshot after movement (smart_move retry, attack auto-approach).
    fn make_executor_with_snapshot_goto() -> (
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
        mock.bind_state(Arc::clone(&state));
        let log = mock.log().clone();
        let executor = CommandExecutor::new(mock, Arc::clone(&state), receiver, None);
        (executor, sender, state, log)
    }

    /// Like `make_executor`, but the mock's `block_interact` places a
    /// "stone" block at the clicked cell + Up (simulating the server
    /// applying azalea's fabricated Up-face hit) — needed by `place_block`
    /// tests that exercise the verification-success path.
    fn make_executor_with_interact_placement() -> (
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
        mock.bind_state(Arc::clone(&state));
        mock.log.interact_places_block.store(true, Ordering::SeqCst);
        let log = mock.log().clone();
        let executor = CommandExecutor::new(mock, Arc::clone(&state), receiver, None);
        (executor, sender, state, log)
    }

    /// Seed the snapshot for a `place_block` test at `pos`: the player
    /// standing right next to it (so the auto-approach is skipped), the
    /// click target (`pos − Up`) present as a solid floor, and the effect
    /// cell `pos` itself ABSENT (empty). `update_snapshot` stores the
    /// snapshot as-is, so the `block_index` is seeded explicitly.
    fn seed_placement_snapshot(state: &SharedState, pos: BlockPos) {
        let click_target = BlockPos::new(pos.x, pos.y - 1, pos.z);
        let mut snapshot = make_populated_snapshot_defaults();
        // Standing one block away — inside the 4.5-block interaction reach,
        // so the auto-approach is skipped and the test world needs no
        // standable-neighbour scaffolding.
        snapshot.self_player.position = BlockPos::new(pos.x - 1, pos.y, pos.z);
        snapshot.self_player.inventory = Vec::new();
        snapshot.blocks = vec![BlockEntry {
            position: click_target,
            block_type: "stone".into(),
            block_state: None,
        }];
        snapshot.block_index = snapshot
            .blocks
            .iter()
            .enumerate()
            .map(|(i, b)| (b.position, i))
            .collect();
        state.update_snapshot(snapshot);
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
        state.update_snapshot(make_populated_snapshot_defaults());
    }

    /// Defaults shared by snapshot-seeded tests: a stone block, a zombie
    /// (entity 42) at (3,64,1), and a player at (0,64,0).
    fn make_populated_snapshot_defaults() -> crate::types::WorldSnapshot {
        crate::types::WorldSnapshot {
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
                position_precise: None,
                yaw: None,
            },
            timestamp: 1,
            chunk_summary: vec![(0, 0), (1, 0)],
            commands_enabled: None,
            ..Default::default()
        }
    }

    // ═══════════════════════════════════════════════════════════════
    // Construction tests
    // ═══════════════════════════════════════════════════════════════

    #[test]
    fn test_new_constructs() {
        let (_executor, _sender, _state, _log) = make_executor();
    }

    // ═══════════════════════════════════════════════════════════════
    // Run-stats bookkeeping (A2)
    // ═══════════════════════════════════════════════════════════════

    /// A successful dispatch increments `commands_processed` and
    /// `commands_succeeded` (and nothing else).
    #[tokio::test]
    async fn test_dispatch_records_success_in_run_stats() {
        let (executor, _sender, state, _log) = make_executor();
        let result = executor.dispatch(BotCommand::QueryInventory).await;
        assert!(
            result.is_ok(),
            "query should succeed while online: {result:?}"
        );

        let stats = state.read_run_stats();
        assert_eq!(stats.commands_processed.load(Ordering::Relaxed), 1);
        assert_eq!(stats.commands_succeeded.load(Ordering::Relaxed), 1);
        assert_eq!(stats.commands_failed.load(Ordering::Relaxed), 0);
    }

    /// A failed dispatch (bot offline) increments `commands_processed` and
    /// `commands_failed` — the panel reports received commands, so denials
    /// count as failures.
    #[tokio::test]
    async fn test_dispatch_records_failure_in_run_stats() {
        let (executor, _sender, state, _log) = make_executor();
        state.set_online(false);

        let result = executor.dispatch(BotCommand::QueryInventory).await;
        assert!(matches!(result, Err(BotError::Offline(_))));

        let stats = state.read_run_stats();
        assert_eq!(stats.commands_processed.load(Ordering::Relaxed), 1);
        assert_eq!(stats.commands_succeeded.load(Ordering::Relaxed), 0);
        assert_eq!(stats.commands_failed.load(Ordering::Relaxed), 1);
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

    #[tokio::test]
    async fn test_move_to_timeout_returns_structured_partial_result() {
        // A stalled pathfinder must NOT surface a bare CommandTimeout — the
        // executor returns a structured partial result (current position +
        // distance) so the MCP client can decide whether to retry/cancel.
        let config = AppConfig {
            command_timeout_secs: 1,
            ..AppConfig::default()
        };
        let state = Arc::new(SharedState::new(config));
        let (sender, receiver) = create_command_channel(16, Arc::clone(&state));
        state.set_online(true);
        let mock = MockBotClient::new();
        mock.log.goto_hangs.store(true, Ordering::SeqCst);
        let log = mock.log().clone();
        let executor = CommandExecutor::new(mock, Arc::clone(&state), receiver, None);
        let handle = spawn_executor(executor);

        let pos = BlockPos::new(100, 64, 200);
        let result = send_and_await(&sender, BotCommand::MoveTo(pos)).await;

        let br = result.expect("timed-out movement returns a BotResult, not an error");
        assert!(!br.success, "timeout must report success=false");
        let data = br.data.expect("timeout result must carry structured data");
        assert_eq!(data["reason"], "timeout");
        assert_eq!(data["target"], serde_json::json!([100, 64, 200]));
        assert!(data["distance_moved"].is_number());
        assert!(br.message.contains("timed out"), "message: {}", br.message);

        drop(sender);
        handle.await.expect("executor should finish");

        // The pathfinder must have been stopped after the timeout.
        assert_eq!(
            log.stop_pathfinding_calls.load(Ordering::SeqCst),
            1,
            "stop_pathfinding must be called once on timeout"
        );
    }

    #[tokio::test]
    async fn test_smart_move_timeout_returns_structured_partial_result() {
        let config = AppConfig {
            command_timeout_secs: 1,
            ..AppConfig::default()
        };
        let state = Arc::new(SharedState::new(config));
        let (sender, receiver) = create_command_channel(16, Arc::clone(&state));
        state.set_online(true);
        let mock = MockBotClient::new();
        mock.log.goto_hangs.store(true, Ordering::SeqCst);
        let log = mock.log().clone();
        let executor = CommandExecutor::new(mock, Arc::clone(&state), receiver, None);
        let handle = spawn_executor(executor);

        let pos = BlockPos::new(50, 64, 50);
        let result = send_and_await(&sender, BotCommand::SmartMove(pos)).await;

        let br = result.expect("timed-out smart_move returns a BotResult");
        assert!(!br.success);
        let data = br.data.expect("timeout result must carry structured data");
        assert_eq!(data["reason"], "timeout");
        assert_eq!(data["target"], serde_json::json!([50, 64, 50]));
        assert!(data["position"].is_array());

        drop(sender);
        handle.await.expect("executor should finish");
        assert_eq!(log.stop_pathfinding_calls.load(Ordering::SeqCst), 1);
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
    async fn test_walk_direction_reports_live_end_position() {
        // R-12 regression: the result data must carry the bot's end
        // position. The live position component is preferred over the
        // snapshot, which can lag a just-finished move by one interval.
        let (executor, sender, state, log) = make_executor();
        make_populated_snapshot(&state);
        *log.live_position.lock().unwrap() = Some([0.5, 64.0, -0.5]);
        let handle = spawn_executor(executor);

        let result = send_and_await(&sender, BotCommand::WalkDirection(Direction::North, 1)).await;
        let br = result.expect("walk should succeed");
        let data = br.data.expect("walk result must carry position data");
        assert_eq!(data["position"], serde_json::json!([0, 64, -1]));

        drop(sender);
        handle.await.expect("executor should finish");
    }

    #[tokio::test]
    async fn test_walk_direction_reports_snapshot_end_position() {
        // When the live position component is unavailable the result falls
        // back to the snapshot; with snapshot-tracking goto this is the
        // post-move target, not the pre-move start.
        let (executor, sender, state, _log) = make_executor_with_snapshot_goto();
        make_populated_snapshot(&state);
        let handle = spawn_executor(executor);

        let result = send_and_await(&sender, BotCommand::WalkDirection(Direction::East, 3)).await;
        let br = result.expect("walk should succeed");
        let data = br.data.expect("walk result must carry position data");
        assert_eq!(data["position"], serde_json::json!([3, 64, 0]));

        drop(sender);
        handle.await.expect("executor should finish");
    }

    #[tokio::test]
    async fn test_walk_direction_uses_margin_wrapper() {
        // M-5: walk_direction previously ran a bare `goto` (same timeout as
        // the envelope, no MOVEMENT_REPLY_MARGIN), so a movement that hit the
        // deadline was reported by `send_command` as Err(CommandTimeout).
        // Routing through `goto_with_margin` turns the deadline into a
        // structured BotResult (reason: "timeout", position, distance).
        let config = AppConfig {
            command_timeout_secs: 1,
            ..AppConfig::default()
        };
        let state = Arc::new(SharedState::new(config));
        let (sender, receiver) = create_command_channel(16, Arc::clone(&state));
        state.set_online(true);
        let mock = MockBotClient::new();
        mock.log.goto_hangs.store(true, Ordering::SeqCst);
        let log = mock.log().clone();
        let executor = CommandExecutor::new(mock, Arc::clone(&state), receiver, None);
        let handle = spawn_executor(executor);

        let result = send_and_await(&sender, BotCommand::WalkDirection(Direction::North, 1)).await;
        let br = result.expect("walk hitting the deadline returns a BotResult, not CommandTimeout");
        assert!(!br.success, "timeout must report success=false");
        let data = br.data.expect("timeout result carries structured data");
        assert_eq!(data["reason"], "timeout");
        assert!(data["target"].is_array());

        drop(sender);
        handle.await.expect("executor should finish");
        assert_eq!(
            log.stop_pathfinding_calls.load(Ordering::SeqCst),
            1,
            "the pathfinder must be stopped after the timeout"
        );
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
        // Up/Down cannot be translated to a horizontal goto target. The
        // executor must reject them with `InvalidParams` (a client-input
        // error), NOT `Internal` — the direction was supplied by the caller
        // and the rejection is deterministic, not an internal failure.
        let (executor, sender, _state, _log) = make_executor();
        let handle = spawn_executor(executor);

        let result = send_and_await(&sender, BotCommand::WalkDirection(Direction::Up, 1)).await;
        assert!(
            matches!(result, Err(BotError::InvalidParams(ref msg))
                if msg.contains("not supported for distance-based movement")),
            "expected InvalidParams for Up, got: {result:?}"
        );

        drop(sender);
        handle.await.expect("executor should finish");
    }

    #[tokio::test]
    async fn test_walk_down_rejected_with_invalid_params() {
        // Down is rejected exactly like Up — vertical directions are not
        // supported for distance-based movement.
        let (executor, sender, _state, _log) = make_executor();
        let handle = spawn_executor(executor);

        let result = send_and_await(&sender, BotCommand::WalkDirection(Direction::Down, 1)).await;
        assert!(
            matches!(result, Err(BotError::InvalidParams(ref msg))
                if msg.contains("north/south/east/west")),
            "expected InvalidParams for Down, got: {result:?}"
        );

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
        // Teleport now routes through the server-authoritative `/tp` command
        // (the old local-ECS position mutation was silently reverted by the
        // server every tick). The mock records the chat command.
        let (executor, sender, _state, log) = make_executor();
        let handle = spawn_executor(executor);

        let pos = BlockPos::new(50, 70, 100);
        let result = send_and_await(&sender, BotCommand::Teleport(pos)).await;
        let br = result.expect("teleport via /tp reports success");
        assert!(br.success);
        assert!(
            br.message.contains("Teleported to (50, 70, 100)"),
            "message: {}",
            br.message
        );

        drop(sender);
        handle.await.expect("executor should finish");

        let chats = log.chat_calls.lock().unwrap();
        assert_eq!(chats.as_slice(), &["/tp 50 70 100"]);
    }

    #[tokio::test]
    async fn test_teleport_rejected_without_op() {
        // The server rejects /tp when the bot lacks operator permissions —
        // the executor must surface `CommandRejected` instead of a fake
        // success (the old implementation always reported success even
        // though the server reverted the position every tick).
        let (executor, _sender, state, log) = make_executor();
        let state2 = Arc::clone(&state);
        let sim = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            state2.add_chat_message(
                "System".into(),
                "You do not have permission to use this command".into(),
            );
            state2.add_chat_message("System".into(), "/tp 50 70 100<--[HERE]".into());
        });

        let result = executor.handle_teleport(BlockPos::new(50, 70, 100)).await;
        assert!(
            matches!(result, Err(BotError::CommandRejected { ref feedback, .. }) if feedback.contains("do not have permission")),
            "expected CommandRejected, got: {result:?}"
        );
        sim.await.expect("simulation task should finish");
        let chats = log.chat_calls.lock().unwrap();
        assert_eq!(chats.as_slice(), &["/tp 50 70 100"]);
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

    #[test]
    fn test_switch_hotbar_slot_out_of_range_direct_handler() {
        // Defense-in-depth: the central `validate_command` gate in dispatch
        // already rejects slot > 8, but the handler's own range check must
        // carry the correct variant (`InvalidParams`, not `Internal`) in case
        // the handler is ever reached directly (e.g. internal dispatch).
        let (executor, _sender, _state, log) = make_executor();

        let result = executor.handle_switch_hotbar_slot(9);
        assert!(
            matches!(result, Err(BotError::InvalidParams(ref msg))
                if msg.contains("out of range")),
            "expected InvalidParams from handler, got: {result:?}"
        );

        // No hotbar switch must have been performed.
        assert!(log.hotbar_switch_calls.lock().unwrap().is_empty());
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

    #[tokio::test]
    async fn test_drop_item_verified_removes_from_slot() {
        let (executor, sender, _state, log) = make_executor();
        {
            let mut inv = log.inventory.lock().unwrap();
            inv.resize(9, None);
            inv[2] = Some(ItemStack {
                item_id: "dirt".into(),
                count: 10,
            });
        }
        let handle = spawn_executor(executor);

        let result = send_and_await(&sender, BotCommand::DropItem(2, 5)).await;
        let br = result.expect("drop should succeed");
        assert!(br.success, "verified drop must report success");
        let data = br.data.expect("drop result must carry verification data");
        assert_eq!(data["verified"], true);
        assert_eq!(data["removed"], 5);

        drop(sender);
        handle.await.expect("executor should finish");
    }

    #[tokio::test]
    async fn test_drop_item_empty_slot_reports_failure() {
        // Dropping from an already-empty slot must NOT report fake success —
        // the executor now verifies the inventory changed.
        let (executor, sender, _state, _log) = make_executor();
        let handle = spawn_executor(executor);

        let result = send_and_await(&sender, BotCommand::DropItem(2, 1)).await;
        let br = result.expect("empty-slot drop returns a result, not an error");
        assert!(
            !br.success,
            "dropping from an empty slot must report failure"
        );
        assert!(br.message.contains("already empty"));

        drop(sender);
        handle.await.expect("executor should finish");
    }

    #[tokio::test]
    async fn test_move_item_to_hotbar_success() {
        let (executor, sender, _state, log) = make_executor();
        {
            let mut inv = log.inventory.lock().unwrap();
            inv.resize(36, None);
            // Main inventory slot 10 holds 64 dirt.
            inv[10] = Some(ItemStack {
                item_id: "dirt".into(),
                count: 64,
            });
        }
        let handle = spawn_executor(executor);

        let result =
            send_and_await(&sender, BotCommand::MoveItemToHotbar(0, "dirt".into(), 1)).await;
        let br = result.expect("move should succeed");
        assert!(br.success, "swap must be verified");
        let data = br.data.expect("move result must carry verification data");
        assert_eq!(data["verified"], true);
        assert_eq!(data["source_slot"], 10);
        assert_eq!(data["source_menu_slot"], 10);

        drop(sender);
        handle.await.expect("executor should finish");

        // The mock swap moved the stack into hotbar slot 0.
        let inv = log.inventory.lock().unwrap();
        assert_eq!(inv[0].as_ref().unwrap().item_id, "dirt");
        assert!(inv[10].is_none(), "source slot must be emptied by the swap");
    }

    #[tokio::test]
    async fn test_move_item_to_hotbar_not_found() {
        let (executor, sender, _state, _log) = make_executor();
        let handle = spawn_executor(executor);

        let result = send_and_await(
            &sender,
            BotCommand::MoveItemToHotbar(0, "diamond".into(), 1),
        )
        .await;
        assert!(
            matches!(result, Err(BotError::InvalidParams(ref msg)) if msg.contains("diamond")),
            "missing item must surface InvalidParams"
        );

        drop(sender);
        handle.await.expect("executor should finish");
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
    async fn test_execute_command_rejection_detected() {
        // The server rejects the command via a chat/system message — the
        // executor must surface `CommandRejected` instead of fake success.
        let (executor, _sender, state, log) = make_executor();
        // A message that arrived BEFORE the command must be excluded from the
        // feedback diff (the baseline is taken at dispatch).
        state.add_chat_message("System".into(), "old unrelated feedback".into());

        // Simulate the server replying with a rejection shortly after the
        // command is sent (inside the 400 ms feedback window).
        let state2 = Arc::clone(&state);
        let sim = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            state2.add_chat_message(
                "System".into(),
                "Incorrect argument for command /item replace entity @s hotbar.0 dirt 64<--[HERE]"
                    .into(),
            );
        });

        let result = executor
            .handle_execute_command("/item replace entity @s hotbar.0 dirt 64".into())
            .await;
        assert!(
            matches!(result, Err(BotError::CommandRejected { ref feedback, .. }) if feedback.contains("Incorrect argument")),
            "rejected command must surface CommandRejected, got: {result:?}"
        );
        sim.await.expect("simulation task should finish");
        let chats = log.chat_calls.lock().unwrap();
        assert_eq!(chats.len(), 1);
    }

    #[tokio::test]
    async fn test_execute_command_rejection_detected_two_line_feedback() {
        // Minecraft reports a rejection as TWO System messages: the error
        // title ("Unknown or incomplete command. See below for error") and
        // the command echo ("gamemode<--[HERE]"), which arrives LAST and
        // carries no rejection keyword. The rejection scan must check every
        // message in the window, not just the newest one — the old
        // newest-only selection reported success for a rejected command.
        let (executor, _sender, state, log) = make_executor();
        let state2 = Arc::clone(&state);
        let sim = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            state2.add_chat_message(
                "System".into(),
                "Unknown or incomplete command. See below for error".into(),
            );
            state2.add_chat_message("System".into(), "gamemode<--[HERE]".into());
        });

        let result = executor.handle_execute_command("/gamemode".into()).await;
        assert!(
            matches!(result, Err(BotError::CommandRejected { ref feedback, .. }) if feedback.contains("Unknown or incomplete command")),
            "two-line rejection must surface CommandRejected with the error title, got: {result:?}"
        );
        sim.await.expect("simulation task should finish");
        let chats = log.chat_calls.lock().unwrap();
        assert_eq!(chats.len(), 1);
    }

    #[tokio::test]
    async fn test_execute_command_rejection_detected_unknown_item() {
        // Vanilla rejects an invalid /give item id with
        // "Unknown item 'minecraft:nonexistent_item_xyz'" followed by the
        // command echo carrying `<--[HERE]`. The rejection scan must match
        // the error title even though the keywordless echo lands last —
        // otherwise give_item reports a fake success.
        let (executor, _sender, state, log) = make_executor();
        let state2 = Arc::clone(&state);
        let sim = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            state2.add_chat_message(
                "System".into(),
                "Unknown item 'minecraft:nonexistent_item_xyz'".into(),
            );
            state2.add_chat_message(
                "System".into(),
                "give TestBot minecraft:nonexistent_item_xyz 1<--[HERE]".into(),
            );
        });

        let result = executor
            .handle_execute_command("/give TestBot minecraft:nonexistent_item_xyz 1".into())
            .await;
        assert!(
            matches!(result, Err(BotError::CommandRejected { ref feedback, .. }) if feedback.contains("Unknown item")),
            "unknown-item rejection must surface CommandRejected, got: {result:?}"
        );
        sim.await.expect("simulation task should finish");
        let chats = log.chat_calls.lock().unwrap();
        assert_eq!(chats.len(), 1);
    }

    #[test]
    fn test_is_command_rejection_matches_unknown_item_variants() {
        // Vanilla and common plugin item-id rejection spellings.
        assert!(is_command_rejection(
            "Unknown item 'minecraft:nonexistent_item_xyz'"
        ));
        assert!(is_command_rejection("unknown item"));
        assert!(is_command_rejection("No such item 'foo'"));
        assert!(!is_command_rejection("Gave 1x minecraft:diamond_pickaxe"));
        assert!(!is_command_rejection("Teleported TestBot to 1, 64, 1"));
    }

    #[tokio::test]
    async fn test_execute_command_success_attaches_server_feedback() {
        // An accepted command still produces system feedback ("Teleported
        // ...") — the executor reports success and attaches the feedback.
        let (executor, _sender, state, _log) = make_executor();
        let state2 = Arc::clone(&state);
        let sim = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            state2.add_chat_message("System".into(), "Teleported AI_Bot to 5.5, -58, 5.5".into());
        });

        let result = executor
            .handle_execute_command("/tp @s 5 -58 5".into())
            .await;
        let br = result.expect("accepted command reports success");
        assert!(br.success);
        assert!(
            br.message.contains("server: Teleported"),
            "server feedback must be attached, got: {}",
            br.message
        );
        sim.await.expect("simulation task should finish");
    }

    #[tokio::test]
    async fn test_execute_command_ignores_player_chat_as_feedback() {
        // Only a *player* message arrives — it must not be treated as server
        // feedback (no rejection, no attachment).
        let (executor, _sender, state, _log) = make_executor();
        let state2 = Arc::clone(&state);
        let sim = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            state2.add_chat_message("Notch".into(), "hello everyone".into());
        });

        let result = executor
            .handle_execute_command("/time set day".into())
            .await;
        let br = result.expect("no system feedback → success");
        assert!(br.success);
        assert!(
            !br.message.contains("server:"),
            "player chat must not be treated as server feedback, got: {}",
            br.message
        );
        sim.await.expect("simulation task should finish");
    }

    #[tokio::test]
    async fn test_execute_command_rejection_detected_full_chat_queue() {
        // Regression: the rejection diff used to be index-based
        // (`skip(baseline_len)`), so once the chat deque was full a length
        // baseline always equalled the cap and the post-command scan skipped
        // every new message — rejected commands were reported as successes in
        // real sessions (unit tests with an empty deque never caught it).
        // The cursor-based diff must still see the rejection.
        let (executor, _sender, state, log) = make_executor();
        // Pre-fill the deque past the cap with a mix of player/System chat so
        // the length-based baseline would have been saturated at 50.
        for i in 0..60 {
            state.add_chat_message(format!("User{i}"), format!("chat message {i}"));
        }

        let state2 = Arc::clone(&state);
        let sim = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            state2.add_chat_message(
                "System".into(),
                "Unknown or incomplete command. See below for error".into(),
            );
            state2.add_chat_message("System".into(), "nosuchcmd test<--[HERE]".into());
        });

        let result = executor
            .handle_execute_command("/nosuchcmd test".into())
            .await;
        assert!(
            matches!(result, Err(BotError::CommandRejected { ref feedback, .. }) if feedback.contains("Unknown or incomplete command")),
            "rejection must be detected even with a full chat queue, got: {result:?}"
        );
        sim.await.expect("simulation task should finish");
        let chats = log.chat_calls.lock().unwrap();
        assert_eq!(chats.len(), 1);
    }

    #[tokio::test]
    async fn test_execute_command_success_feedback_full_chat_queue() {
        // Same full-queue pressure, but the success path: the newest System
        // feedback after the cursor must still be attached.
        let (executor, _sender, state, _log) = make_executor();
        for i in 0..60 {
            state.add_chat_message(format!("User{i}"), format!("chat message {i}"));
        }

        let state2 = Arc::clone(&state);
        let sim = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            state2.add_chat_message("System".into(), "Teleported AI_Bot to 5.5, -58, 5.5".into());
        });

        let result = executor
            .handle_execute_command("/tp @s 5 -58 5".into())
            .await;
        let br = result.expect("accepted command reports success with a full queue");
        assert!(br.success);
        assert!(
            br.message.contains("server: Teleported"),
            "server feedback must be attached with a full queue, got: {}",
            br.message
        );
        sim.await.expect("simulation task should finish");
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
        let (executor, sender, state, log) = make_executor();
        // Seed the snapshot with entity 42 (at (3,64,1), within attack reach
        // of the player at (0,64,0)) so the reach check passes.
        make_populated_snapshot(&state);
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
        let (executor, sender, state, log) = make_executor();
        log.attack_succeeds.store(false, Ordering::SeqCst);
        // Seed entity 99 in the snapshot so the reach check passes and the
        // underlying attack failure is what surfaces.
        state.update_snapshot(crate::types::WorldSnapshot {
            entities: vec![EntityEntry {
                id: 99,
                uuid: "target-99".into(),
                entity_type: "zombie".into(),
                position: BlockPos::new(3, 64, 1),
                display_name: Some("Zombie".into()),
                health: Some(20.0),
            }],
            ..make_populated_snapshot_defaults()
        });
        let handle = spawn_executor(executor);

        let result = send_and_await(&sender, BotCommand::AttackEntity(99)).await;
        assert!(result.is_err());

        drop(sender);
        handle.await.expect("executor should finish");
    }

    /// An entity absent from the snapshot is rejected as `InvalidParams`
    /// before any attack attempt (defence-in-depth; the MCP layer already
    /// enforces the same existence check).
    #[tokio::test]
    async fn test_attack_entity_missing_from_snapshot_rejected() {
        let (executor, sender, state, log) = make_executor();
        make_populated_snapshot(&state); // contains entity 42 only
        let handle = spawn_executor(executor);

        let result = send_and_await(&sender, BotCommand::AttackEntity(999)).await;
        assert!(matches!(result, Err(BotError::InvalidParams(_))));

        drop(sender);
        handle.await.expect("executor should finish");

        assert!(
            log.attack_calls.lock().unwrap().is_empty(),
            "no attack may be issued for a target missing from the snapshot"
        );
    }

    #[tokio::test]
    async fn test_attack_entity_auto_approaches_when_too_far() {
        // Moving-target fix: a target farther than MAX_ATTACK_REACH must be
        // approached (goto) before attacking; with the mock goto updating the
        // shared snapshot, the fresh re-check passes and the attack lands.
        let (executor, sender, state, log) = make_executor_with_snapshot_goto();
        state.update_snapshot(crate::types::WorldSnapshot {
            entities: vec![EntityEntry {
                id: 42,
                uuid: "far-42".into(),
                entity_type: "zombie".into(),
                position: BlockPos::new(20, 64, 0),
                display_name: Some("Zombie".into()),
                health: Some(20.0),
            }],
            ..make_populated_snapshot_defaults()
        });
        let handle = spawn_executor(executor);

        let result = send_and_await(&sender, BotCommand::AttackEntity(42)).await;
        let br = result.expect("expected Ok(BotResult)");
        assert!(br.success, "auto-approach should enable the attack: {br:?}");
        assert!(
            br.message.contains("approaching"),
            "message: {}",
            br.message
        );

        drop(sender);
        handle.await.expect("executor should finish");
        assert_eq!(log.goto_calls.lock().unwrap().len(), 1);
        let attacks = log.attack_calls.lock().unwrap();
        assert_eq!(attacks.len(), 1);
        assert_eq!(attacks[0], 42);
    }

    #[tokio::test]
    async fn test_attack_entity_too_far_reports_too_far_after_approach() {
        // When the approach does not bring the bot within reach (the mock
        // does not update the snapshot), the handler must still fail honestly
        // with TooFar and must not attack.
        let (executor, sender, state, log) = make_executor();
        state.update_snapshot(crate::types::WorldSnapshot {
            entities: vec![EntityEntry {
                id: 42,
                uuid: "far-42".into(),
                entity_type: "zombie".into(),
                position: BlockPos::new(20, 64, 0),
                display_name: Some("Zombie".into()),
                health: Some(20.0),
            }],
            ..make_populated_snapshot_defaults()
        });
        let handle = spawn_executor(executor);

        let result = send_and_await(&sender, BotCommand::AttackEntity(42)).await;
        assert!(
            matches!(result, Err(BotError::TooFar { .. })),
            "expected TooFar, got {result:?}"
        );

        drop(sender);
        handle.await.expect("executor should finish");
        assert_eq!(
            log.goto_calls.lock().unwrap().len(),
            1,
            "one approach attempt"
        );
        assert!(
            log.attack_calls.lock().unwrap().is_empty(),
            "no attack when still out of reach"
        );
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
        // H-2 semantics: `pos` is the cell the placed block OCCUPIES. The
        // executor right-clicks the cell below (`pos − Up`, azalea's
        // fabricated Up-face hit) and verifies the block appeared. With the
        // mock's placement simulation the block lands at `pos` and the
        // result is a verified success — never unconditional.
        let (executor, sender, state, log) = make_executor_with_interact_placement();
        let pos = BlockPos::new(10, 64, 20);
        seed_placement_snapshot(&state, pos);
        let handle = spawn_executor(executor);

        let result = send_and_await(&sender, BotCommand::PlaceBlock(pos, "stone".into())).await;
        let br = result.expect("expected Ok(BotResult)");
        assert!(br.success, "verified placement must succeed: {br:?}");
        assert!(br.message.contains("Placed stone at"));
        assert!(
            !br.message.contains("slot:"),
            "message must not contain 'slot:' prefix: {}",
            br.message
        );

        drop(sender);
        handle.await.expect("executor should finish");

        let interacts = log.interact_calls.lock().unwrap();
        assert_eq!(interacts.len(), 1);
        // The clicked cell is BELOW `pos` — the Up-face convention puts the
        // block at `pos`.
        assert_eq!(interacts[0], BlockPos::new(10, 63, 20));
        // No slot: prefix → no hotbar switch.
        assert!(log.hotbar_switch_calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_place_block_selects_slot_from_prefix() {
        // The MCP layer encodes the hotbar slot as "slot:N" in the block_type
        // field; the executor must select that slot before interacting.
        // The result message must strip the "slot:" prefix.
        let (executor, sender, state, log) = make_executor_with_interact_placement();
        let pos = BlockPos::new(10, 64, 20);
        seed_placement_snapshot(&state, pos);
        let handle = spawn_executor(executor);

        let result = send_and_await(&sender, BotCommand::PlaceBlock(pos, "slot:3".into())).await;
        let br = result.expect("expected Ok(BotResult)");
        assert!(br.success, "verified placement must succeed: {br:?}");
        assert!(br.message.contains("Placed 3 at"));
        assert!(
            !br.message.contains("slot:"),
            "message must not contain 'slot:' prefix: {}",
            br.message
        );

        drop(sender);
        handle.await.expect("executor should finish");

        let slots = log.hotbar_switch_calls.lock().unwrap();
        assert_eq!(slots.len(), 1);
        assert_eq!(slots[0], 3);

        let interacts = log.interact_calls.lock().unwrap();
        assert_eq!(interacts.len(), 1);
        assert_eq!(interacts[0], BlockPos::new(10, 63, 20));
    }

    #[tokio::test]
    async fn test_place_block_rejects_malformed_slot_encoding() {
        // F2-5: a "slot:" prefix whose payload is not a u8 is a malformed
        // internal encoding. Previously the executor warn-and-continued with
        // the held item and still reported success — dishonest. It must fail
        // with InvalidParams and NOT interact.
        let (executor, sender, _state, log) = make_executor();
        let handle = spawn_executor(executor);

        let pos = BlockPos::new(10, 64, 20);
        let result = send_and_await(&sender, BotCommand::PlaceBlock(pos, "slot:abc".into())).await;
        assert!(
            matches!(result, Err(BotError::InvalidParams(ref msg))
                if msg.contains("invalid internal slot encoding")),
            "expected InvalidParams for slot:abc, got: {result:?}"
        );

        drop(sender);
        handle.await.expect("executor should finish");

        // No interaction and no hotbar switch may have happened.
        assert!(log.interact_calls.lock().unwrap().is_empty());
        assert!(log.hotbar_switch_calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_place_block_rejects_out_of_range_slot_encoding() {
        // F2-5: "slot:9" parses but is outside the hotbar (0-8). Same
        // contract as the malformed case: honest InvalidParams, no interact.
        let (executor, sender, _state, log) = make_executor();
        let handle = spawn_executor(executor);

        let pos = BlockPos::new(10, 64, 20);
        let result = send_and_await(&sender, BotCommand::PlaceBlock(pos, "slot:9".into())).await;
        assert!(
            matches!(result, Err(BotError::InvalidParams(ref msg))
                if msg.contains("invalid internal slot encoding")),
            "expected InvalidParams for slot:9, got: {result:?}"
        );

        drop(sender);
        handle.await.expect("executor should finish");

        assert!(log.interact_calls.lock().unwrap().is_empty());
        assert!(log.hotbar_switch_calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_place_block_verifies_effect_cell() {
        // H-2: the executor must NEVER report unconditional success. With the
        // mock's block_interact being a no-op (the server never confirms the
        // placement), the result is an honest success:false naming the block
        // type and the effect cell — the historical fake-success bug.
        let (executor, sender, state, log) = make_executor();
        let pos = BlockPos::new(5, 64, 0);
        seed_placement_snapshot(&state, pos);
        // Tiny verification budget so the timeout path is fast.
        state.update_config(|c| c.snapshot_interval_ms = 1);
        let handle = spawn_executor(executor);

        let result = send_and_await(&sender, BotCommand::PlaceBlock(pos, "stone".into())).await;
        let br = result.expect("expected Ok(BotResult)");
        assert!(
            !br.success,
            "no block appeared → honest failure, got: {br:?}"
        );
        assert!(
            br.message.contains("stone"),
            "message names the block: {}",
            br.message
        );
        assert!(
            br.message.contains("(5, 64, 0)"),
            "message names the cell: {}",
            br.message
        );

        drop(sender);
        handle.await.expect("executor should finish");

        let interacts = log.interact_calls.lock().unwrap();
        assert_eq!(interacts.len(), 1, "the click must still have been sent");
    }

    #[tokio::test]
    async fn test_place_block_occupied_target_rejected() {
        // H-2: an occupied effect cell is rejected as InvalidParams before
        // any interaction — the historical fire-and-forget success is gone.
        let (executor, sender, state, log) = make_executor();
        let pos = BlockPos::new(5, 64, 0);
        let mut snapshot = make_populated_snapshot_defaults();
        snapshot.self_player.position = BlockPos::new(4, 64, 0);
        snapshot.blocks = vec![
            BlockEntry {
                position: BlockPos::new(5, 63, 0),
                block_type: "stone".into(),
                block_state: None,
            },
            BlockEntry {
                position: pos,
                block_type: "dirt".into(),
                block_state: None,
            },
        ];
        snapshot.block_index = snapshot
            .blocks
            .iter()
            .enumerate()
            .map(|(i, b)| (b.position, i))
            .collect();
        state.update_snapshot(snapshot);
        let handle = spawn_executor(executor);

        let result = send_and_await(&sender, BotCommand::PlaceBlock(pos, "stone".into())).await;
        assert!(
            matches!(result, Err(BotError::InvalidParams(ref msg)) if msg.contains("occupied")),
            "expected InvalidParams 'already occupied', got: {result:?}"
        );

        drop(sender);
        handle.await.expect("executor should finish");
        assert!(
            log.interact_calls.lock().unwrap().is_empty(),
            "no interact may be issued when the target cell is occupied"
        );
    }

    #[tokio::test]
    async fn test_place_block_clicks_cell_below() {
        // H-2: because azalea's block_interact fabricates an Up-face hit, the
        // executor must right-click `pos − Up` so the placed block lands at
        // `pos`. The recorded interact position must be the cell below.
        let (executor, sender, state, log) = make_executor();
        let pos = BlockPos::new(5, 64, 0);
        seed_placement_snapshot(&state, pos);
        state.update_config(|c| c.snapshot_interval_ms = 1);
        let handle = spawn_executor(executor);

        let result = send_and_await(&sender, BotCommand::PlaceBlock(pos, "stone".into())).await;
        assert!(result.is_ok(), "result: {result:?}");

        drop(sender);
        handle.await.expect("executor should finish");

        let interacts = log.interact_calls.lock().unwrap();
        assert_eq!(interacts.len(), 1);
        assert_eq!(
            interacts[0],
            BlockPos::new(5, 63, 0),
            "must click pos − Up, not pos itself"
        );
    }

    #[tokio::test]
    async fn test_place_block_success_when_block_appears() {
        // H-2: when the server (simulated by the mock placing a block at the
        // clicked cell + Up) confirms the placement, the result is an honest
        // success:true with the block type and position in the message.
        let (executor, sender, state, _log) = make_executor_with_interact_placement();
        let pos = BlockPos::new(5, 64, 0);
        seed_placement_snapshot(&state, pos);
        let handle = spawn_executor(executor);

        let result = send_and_await(&sender, BotCommand::PlaceBlock(pos, "stone".into())).await;
        let br = result.expect("expected Ok(BotResult)");
        assert!(br.success, "placement confirmed → success: {br:?}");
        assert!(
            br.message.contains("Placed stone at (5, 64, 0)"),
            "message: {}",
            br.message
        );
        let data = br.data.expect("success result carries verification data");
        assert_eq!(data["verified"], true);

        drop(sender);
        handle.await.expect("executor should finish");
    }

    #[tokio::test]
    async fn test_place_block_rejects_y_below_floor() {
        // H-2 defense-in-depth: `pos.y = -64` would put the click target
        // (`pos − Up`) at y=-65, outside the world. `validate_command`
        // accepts y=-64 (its bounds are -64..=320), so the handler must
        // reject it — the MCP layer validates too, but internal dispatchers
        // must not wedge the bot.
        let (executor, sender, _state, log) = make_executor();
        let handle = spawn_executor(executor);

        let pos = BlockPos::new(0, -64, 0);
        let result = send_and_await(&sender, BotCommand::PlaceBlock(pos, "stone".into())).await;
        assert!(
            matches!(result, Err(BotError::InvalidParams(ref msg))
                if msg.contains("-63") || msg.contains("below")),
            "y=-64 must be rejected by the handler, got: {result:?}"
        );

        drop(sender);
        handle.await.expect("executor should finish");
        assert!(log.interact_calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_use_item_on_block() {
        // Without an item_slot the bot interacts with the currently held item.
        let (executor, sender, _state, log) = make_executor();
        let handle = spawn_executor(executor);

        let pos = BlockPos::new(5, 65, 5);
        let result = send_and_await(&sender, BotCommand::UseItemOnBlock(pos, None, None)).await;
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
        let result = send_and_await(&sender, BotCommand::UseItemOnBlock(pos, Some(3), None)).await;
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
    async fn test_use_item_on_block_reports_item_used() {
        // The result message must name the item actually held so callers can
        // detect a wrong-slot interaction (the water-bucket smoke failure).
        let (executor, sender, state, _log) = make_executor();
        let mut snapshot = make_populated_snapshot_defaults();
        snapshot.self_player.held_item_slot = 2;
        snapshot.self_player.inventory = vec![InventorySlot {
            slot_index: 2,
            item_id: "water_bucket".into(),
            count: 1,
        }];
        state.update_snapshot(snapshot);
        let handle = spawn_executor(executor);

        let pos = BlockPos::new(5, 65, 5);
        let result = send_and_await(&sender, BotCommand::UseItemOnBlock(pos, None, None)).await;
        let br = result.expect("expected Ok(BotResult)");
        assert!(
            br.message.contains("water_bucket"),
            "message should name the used item: {}",
            br.message
        );

        drop(sender);
        handle.await.expect("executor should finish");
    }

    #[tokio::test]
    async fn test_use_item_on_block_out_of_range_slot_direct_handler() {
        // Defense-in-depth: dispatch's central gate already rejects
        // item_slot > 8, but the handler's own branch must carry
        // `InvalidParams` (not `Internal`) and must not interact.
        let (executor, _sender, _state, log) = make_executor();

        let pos = BlockPos::new(5, 65, 5);
        let result = executor.handle_use_item_on_block(pos, Some(9), None).await;
        assert!(
            matches!(result, Err(BotError::InvalidParams(ref msg))
                if msg.contains("out of hotbar range")),
            "expected InvalidParams from handler, got: {result:?}"
        );

        assert!(log.interact_calls.lock().unwrap().is_empty());
        assert!(log.hotbar_switch_calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_use_item_on_block_bucket_unsupported_error() {
        // Fluid bucket placement is not supported by azalea 0.15.1's
        // fabricated interaction hit — the verification budget expires and
        // the handler must return the targeted "bucket_placement_unsupported"
        // failure (with the /setblock alternative) instead of the generic
        // "interaction was likely rejected" message.
        let (executor, sender, state, _log) = make_executor();
        let mut snapshot = make_populated_snapshot_defaults();
        snapshot.self_player.held_item_slot = 0;
        // Stand next to the interaction target so the auto-approach is
        // skipped — the empty test world has no standable neighbour for the
        // pathfinder, which would otherwise abort the handler early.
        snapshot.self_player.position = BlockPos::new(4, 64, 0);
        snapshot.self_player.inventory = vec![InventorySlot {
            slot_index: 0,
            item_id: "water_bucket".into(),
            count: 1,
        }];
        // `update_snapshot` stores the snapshot as-is (no block_index
        // rebuild), so seed the index for the interaction target + effect
        // cell lookups.
        snapshot.block_index = snapshot
            .blocks
            .iter()
            .enumerate()
            .map(|(i, b)| (b.position, i))
            .collect();
        // Keep the verification budget (snapshot_interval_ms + 250 ms) tiny.
        state.update_config(|c| c.snapshot_interval_ms = 1);
        state.update_snapshot(snapshot);
        let handle = spawn_executor(executor);

        let pos = BlockPos::new(5, 64, 0);
        let effect = BlockPos::new(5, 65, 0);
        let result = send_and_await(
            &sender,
            BotCommand::UseItemOnBlock(pos, Some(0), Some(effect)),
        )
        .await;
        let br = result.expect("expected Ok(BotResult)");
        assert!(!br.success, "bucket placement must report failure");
        assert!(
            br.message.contains("cannot place fluid buckets"),
            "message must explain the azalea limitation, got: {}",
            br.message
        );
        assert!(
            br.message.contains("/setblock"),
            "message must offer the /setblock alternative, got: {}",
            br.message
        );
        let data = br.data.expect("bucket failure carries data");
        assert_eq!(data["reason"], "bucket_placement_unsupported");
        assert_eq!(data["item"], "water_bucket");

        drop(sender);
        handle.await.expect("executor should finish");
    }

    #[tokio::test]
    async fn test_use_item_non_placement_skips_occupancy_check() {
        // M-6: the effect-cell occupancy pre-check (and the auto-approach)
        // only apply to PLACEMENT items. Flint-and-steel has no placement
        // effect, so a ceiling above the click target must NOT produce a
        // false `InvalidParams("target cell already occupied")` — the
        // interaction proceeds and reports the legacy "Used flint_and_steel"
        // success.
        let (executor, sender, state, log) = make_executor();
        let mut snapshot = make_populated_snapshot_defaults();
        snapshot.self_player.held_item_slot = 0;
        snapshot.self_player.position = BlockPos::new(4, 64, 0); // inside reach
        snapshot.self_player.inventory = vec![InventorySlot {
            slot_index: 0,
            item_id: "flint_and_steel".into(),
            count: 1,
        }];
        // The click target (5,64,0) AND the "effect" cell above it (5,65,0)
        // are both solid — the old unconditional pre-check rejected this.
        snapshot.blocks = vec![
            BlockEntry {
                position: BlockPos::new(5, 64, 0),
                block_type: "stone".into(),
                block_state: None,
            },
            BlockEntry {
                position: BlockPos::new(5, 65, 0),
                block_type: "stone".into(),
                block_state: None,
            },
        ];
        snapshot.block_index = snapshot
            .blocks
            .iter()
            .enumerate()
            .map(|(i, b)| (b.position, i))
            .collect();
        state.update_snapshot(snapshot);
        let handle = spawn_executor(executor);

        let pos = BlockPos::new(5, 64, 0);
        let effect = BlockPos::new(5, 65, 0);
        let result = send_and_await(
            &sender,
            BotCommand::UseItemOnBlock(pos, Some(0), Some(effect)),
        )
        .await;
        let br = result
            .expect("non-placement item with an occupied effect cell must NOT be rejected (M-6)");
        assert!(br.success, "interaction proceeds: {br:?}");
        assert!(
            br.message.contains("flint_and_steel"),
            "message names the item: {}",
            br.message
        );

        drop(sender);
        handle.await.expect("executor should finish");
        assert_eq!(log.interact_calls.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn test_use_item_placement_item_occupied_effect_still_rejected() {
        // M-6 guard: gating the occupancy pre-check on
        // `item_has_placement_effect` must NOT disable it for genuine
        // placement items — a stone "block" used on a solid effect cell is
        // still rejected before any interaction.
        let (executor, sender, state, log) = make_executor();
        let mut snapshot = make_populated_snapshot_defaults();
        snapshot.self_player.held_item_slot = 0;
        snapshot.self_player.position = BlockPos::new(4, 64, 0);
        snapshot.self_player.inventory = vec![InventorySlot {
            slot_index: 0,
            item_id: "stone".into(),
            count: 1,
        }];
        snapshot.blocks = vec![
            BlockEntry {
                position: BlockPos::new(5, 64, 0),
                block_type: "stone".into(),
                block_state: None,
            },
            BlockEntry {
                position: BlockPos::new(5, 65, 0),
                block_type: "stone".into(),
                block_state: None,
            },
        ];
        snapshot.block_index = snapshot
            .blocks
            .iter()
            .enumerate()
            .map(|(i, b)| (b.position, i))
            .collect();
        state.update_snapshot(snapshot);
        let handle = spawn_executor(executor);

        let pos = BlockPos::new(5, 64, 0);
        let effect = BlockPos::new(5, 65, 0);
        let result = send_and_await(
            &sender,
            BotCommand::UseItemOnBlock(pos, Some(0), Some(effect)),
        )
        .await;
        assert!(
            matches!(result, Err(BotError::InvalidParams(ref msg)) if msg.contains("occupied")),
            "placement item with an occupied effect cell must be rejected, got: {result:?}"
        );

        drop(sender);
        handle.await.expect("executor should finish");
        assert!(log.interact_calls.lock().unwrap().is_empty());
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

    #[tokio::test]
    async fn test_take_from_container_inventory_full_rejected() {
        // F6-3: with all 36 player-inventory slots occupied (read live
        // from the open menu), the take must fail fast with InventoryFull
        // instead of shift-clicking a stack that would be dropped.
        let (executor, sender, _state, log) = make_executor();
        log.player_inventory_occupied.store(36, Ordering::SeqCst);
        let handle = spawn_executor(executor);

        let result = send_and_await(&sender, BotCommand::TakeFromContainer(3, 10)).await;
        assert!(matches!(result, Err(BotError::InventoryFull)));

        drop(sender);
        handle.await.expect("executor should finish");
    }

    #[tokio::test]
    async fn test_take_from_container_guard_passes_with_free_slots() {
        // F6-3 regression: the old snapshot-based guard could never fire
        // (the snapshot inventory is always empty while a container menu
        // is open), and a live read with free slots must not reject. With
        // 35 occupied slots the handler must pass the guard and proceed to
        // the container-open gate (which errors here only because no real
        // ContainerHandle can be constructed in unit tests).
        let (executor, sender, _state, log) = make_executor();
        log.player_inventory_occupied.store(35, Ordering::SeqCst);
        let handle = spawn_executor(executor);

        let result = send_and_await(&sender, BotCommand::TakeFromContainer(3, 10)).await;
        assert!(
            matches!(&result, Err(BotError::Internal(msg)) if msg.contains("no container")),
            "guard must not fire with a free slot; got: {result:?}"
        );

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
    async fn test_equip_tool_moves_from_main_inventory() {
        // A tool that lives only in the main inventory (slot 9-35) must be
        // auto-moved into the first free hotbar slot, then switched to — not
        // rejected with "move it to a hotbar slot first".
        let (executor, sender, _state, log) = make_executor();
        {
            let mut inv = log.inventory.lock().unwrap();
            inv.resize(36, None);
            // Main inventory slot 12 holds an iron pickaxe.
            inv[12] = Some(ItemStack {
                item_id: "iron_pickaxe".into(),
                count: 1,
            });
        }
        let handle = spawn_executor(executor);

        let result = send_and_await(&sender, BotCommand::EquipTool(ToolType::Pickaxe)).await;
        let br = result.expect("equip from main inventory should succeed");
        assert!(br.success, "message: {}", br.message);
        assert!(br.message.contains("moved to hotbar slot 0"));

        drop(sender);
        handle.await.expect("executor should finish");

        // The tool must have been swapped into hotbar slot 0 and switched to.
        let inv = log.inventory.lock().unwrap();
        assert_eq!(inv[0].as_ref().unwrap().item_id, "iron_pickaxe");
        let slots = log.hotbar_switch_calls.lock().unwrap();
        assert_eq!(slots.as_slice(), &[0u8]);
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
    // SmartMove honesty tests (F2-6)
    // ═══════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_smart_move_blocked_returns_failure() {
        // F2-6: when the pathfinder fails (goto Err), the result must carry
        // `success: false` — previously both blocked branches reported
        // `success: true` while admitting the move was blocked.
        let (executor, sender, _state, _log) = make_executor();
        _log.goto_succeeds.store(false, Ordering::SeqCst);
        let handle = spawn_executor(executor);

        let target = BlockPos::new(5, 64, 5);
        let result = send_and_await(&sender, BotCommand::SmartMove(target)).await;
        assert!(
            result.is_ok(),
            "blocked smart_move is Ok(BotResult): {result:?}"
        );
        let br = result.unwrap();
        assert!(
            !br.success,
            "expected success == false when pathfinding is blocked"
        );
        assert!(br.message.contains("blocked"), "message: {}", br.message);

        // The structured payload must stay honest: reached == false.
        let data = br.data.expect("data present");
        assert_eq!(data.get("reached"), Some(&serde_json::json!(false)));

        drop(sender);
        handle.await.expect("executor should finish");
    }

    #[tokio::test]
    async fn test_smart_move_obstacle_carries_object_with_coordinates() {
        // Bug #4: the obstacle field used to be null (or a bare string).
        // It must now be an object { block_type, x, y, z } so the MCP client
        // can see exactly what (and where) blocked the bot.
        use std::collections::HashMap;
        let (executor, sender, state, log) = make_executor();
        log.goto_succeeds.store(false, Ordering::SeqCst);
        // Player at (0,64,0), stone directly on the line to the target at
        // (5,64,0). The literal must carry a populated block_index — the
        // production SnapshotBuilder derives it, but test literals do not.
        let mut block_index = HashMap::new();
        block_index.insert(BlockPos::new(5, 64, 0), 0usize);
        state.update_snapshot(crate::types::WorldSnapshot {
            blocks: vec![BlockEntry {
                position: BlockPos::new(5, 64, 0),
                block_type: "stone".into(),
                block_state: None,
            }],
            block_index,
            self_player: SelfPlayer {
                position: BlockPos::new(0, 64, 0),
                ..Default::default()
            },
            ..Default::default()
        });
        let handle = spawn_executor(executor);

        let target = BlockPos::new(5, 64, 0);
        let result = send_and_await(&sender, BotCommand::SmartMove(target)).await;
        let br = result.expect("blocked smart_move is Ok(BotResult)");
        assert!(!br.success);

        let data = br.data.expect("data present");
        let obstacle = data.get("obstacle").expect("obstacle must be present");
        assert!(
            obstacle.is_object(),
            "obstacle must be an object, got: {obstacle}"
        );
        assert_eq!(obstacle["block_type"], "stone");
        assert_eq!(obstacle["x"], 5);
        assert_eq!(obstacle["y"], 64);
        assert_eq!(obstacle["z"], 0);

        drop(sender);
        handle.await.expect("executor should finish");
    }

    #[tokio::test]
    async fn test_smart_move_unreached_returns_failure() {
        // F2-6: goto returning Ok but the bot ending up far from the target
        // (obstacle) must ALSO be `success: false` with reached == false.
        // The mock's successful goto does not move the snapshot position
        // (default (0,0,0)), so target (5,64,5) is never "reached".
        let (executor, sender, _state, _log) = make_executor();
        let handle = spawn_executor(executor);

        let target = BlockPos::new(5, 64, 5);
        let result = send_and_await(&sender, BotCommand::SmartMove(target)).await;
        assert!(result.is_ok(), "expected Ok(BotResult): {result:?}");
        let br = result.unwrap();
        assert!(
            !br.success,
            "expected success == false when target not reached"
        );
        let data = br.data.expect("data present");
        assert_eq!(data.get("reached"), Some(&serde_json::json!(false)));
        assert_eq!(data.get("reason"), Some(&serde_json::json!("obstacle")));

        drop(sender);
        handle.await.expect("executor should finish");
    }

    #[tokio::test]
    async fn test_smart_move_reached_success() {
        // Happy path stays honest: snapshot position at the target means
        // reached == true and success == true.
        let (executor, sender, state, _log) = make_executor();
        make_populated_snapshot(&state); // self_player.position = (0, 64, 0)
        let handle = spawn_executor(executor);

        let target = BlockPos::new(0, 64, 0);
        let result = send_and_await(&sender, BotCommand::SmartMove(target)).await;
        assert!(result.is_ok(), "expected Ok(BotResult): {result:?}");
        let br = result.unwrap();
        assert!(br.success, "expected success == true when reached");
        let data = br.data.expect("data present");
        assert_eq!(data.get("reached"), Some(&serde_json::json!(true)));
        assert_eq!(data.get("retried"), Some(&serde_json::json!(false)));

        drop(sender);
        handle.await.expect("executor should finish");
    }

    #[tokio::test]
    async fn test_smart_move_uses_live_position() {
        // M-4: the reached computation must use the bot's LIVE position
        // (zero-wait), not the throttled snapshot — which can lag a long
        // move by a whole idle interval (5 s) and made a physically-
        // completed move look unreached, so smart_move reported a false
        // "obstacle". The snapshot player stays at (0,64,0) while the live
        // client position says the bot already arrived at the target.
        let (executor, sender, state, log) = make_executor();
        make_populated_snapshot(&state);
        *log.live_position.lock().unwrap() = Some([5.0, 64.0, 5.0]);
        let handle = spawn_executor(executor);

        let target = BlockPos::new(5, 64, 5);
        let result = send_and_await(&sender, BotCommand::SmartMove(target)).await;
        let br = result.expect("expected Ok(BotResult): {result:?}");
        assert!(
            br.success,
            "live position at the target must count as reached: {br:?}"
        );
        let data = br.data.expect("data present");
        assert_eq!(data.get("reached"), Some(&serde_json::json!(true)));
        assert_eq!(data.get("position"), Some(&serde_json::json!([5, 64, 5])));
        assert_eq!(data.get("retried"), Some(&serde_json::json!(false)));

        drop(sender);
        handle.await.expect("executor should finish");
    }

    #[tokio::test]
    async fn test_smart_move_retries_once_and_reaches_target() {
        // Transient-obstacle regression: the first goto attempt fails with a
        // pathfinding error, the single retry succeeds and lands the snapshot
        // position on the target — result must report reached + retried and
        // issue exactly two goto calls.
        let (executor, sender, state, log) = make_executor_with_snapshot_goto();
        make_populated_snapshot(&state); // player at (0,64,0)
        log.goto_failures_remaining.store(1, Ordering::SeqCst);
        let handle = spawn_executor(executor);

        let target = BlockPos::new(5, 64, 0);
        let result = send_and_await(&sender, BotCommand::SmartMove(target)).await;
        let br = result.expect("expected Ok(BotResult)");
        assert!(br.success, "retry should reach the target: {br:?}");
        let data = br.data.expect("data present");
        assert_eq!(data.get("reached"), Some(&serde_json::json!(true)));
        assert_eq!(data.get("retried"), Some(&serde_json::json!(true)));

        drop(sender);
        handle.await.expect("executor should finish");
        assert_eq!(log.goto_calls.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn test_smart_move_retries_once_then_reports_obstacle() {
        // Both attempts fail: exactly two goto calls, and the final result
        // stays honest (success false, reached false, retried true).
        let (executor, sender, _state, log) = make_executor();
        log.goto_succeeds.store(false, Ordering::SeqCst);
        let handle = spawn_executor(executor);

        let target = BlockPos::new(5, 64, 5);
        let result = send_and_await(&sender, BotCommand::SmartMove(target)).await;
        let br = result.expect("expected Ok(BotResult)");
        assert!(!br.success, "expected success == false: {br:?}");
        let data = br.data.expect("data present");
        assert_eq!(data.get("reached"), Some(&serde_json::json!(false)));
        assert_eq!(data.get("retried"), Some(&serde_json::json!(true)));

        drop(sender);
        handle.await.expect("executor should finish");
        assert_eq!(log.goto_calls.lock().unwrap().len(), 2);
    }

    // ═══════════════════════════════════════════════════════════════
    // Canonical inventory slot ordering tests
    // ═══════════════════════════════════════════════════════════════

    #[test]
    fn test_canonical_inventory_slot_reorders_hotbar_first() {
        // azalea order: main inventory 0-26, hotbar 27-35.
        // canonical order: hotbar 0-8, main inventory 9-35.
        assert_eq!(canonical_inventory_slot(0), 9);
        assert_eq!(canonical_inventory_slot(26), 35);
        assert_eq!(canonical_inventory_slot(27), 0);
        assert_eq!(canonical_inventory_slot(35), 8);
    }

    // ═══════════════════════════════════════════════════════════════
    // FlyTo tests
    // ═══════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_fly_to_vertical_reaches_target() {
        // fly_to must handle a target whose Y differs from the current Y:
        // horizontal goto first (same Y), then a direct position update.
        let (executor, sender, state, log) = make_executor();
        state.update_snapshot(crate::types::WorldSnapshot {
            self_player: SelfPlayer {
                position: BlockPos::new(0, 64, 0),
                gamemode: GameMode::Creative,
                ..Default::default()
            },
            ..Default::default()
        });
        let handle = spawn_executor(executor);

        let target = BlockPos::new(10, 70, 0);
        let result = send_and_await(&sender, BotCommand::FlyTo(target)).await;
        let br = result.expect("fly_to should succeed");
        assert!(br.success, "message: {}", br.message);
        let data = br.data.expect("data present");
        assert_eq!(data.get("reached"), Some(&serde_json::json!(true)));
        assert_eq!(data.get("reason"), Some(&serde_json::json!("reached")));
        assert_eq!(data.get("position"), Some(&serde_json::json!([10, 70, 0])));

        drop(sender);
        handle.await.expect("executor should finish");

        // Horizontal leg keeps Y at 64; the vertical leg issues a
        // server-authoritative `/tp` to the exact target.
        let gotos = log.goto_calls.lock().unwrap();
        assert_eq!(gotos.as_slice(), &[BlockPos::new(10, 64, 0)]);
        let chats = log.chat_calls.lock().unwrap();
        assert_eq!(chats.as_slice(), &["/tp 10 70 0"]);
    }

    #[tokio::test]
    async fn test_fly_to_blocked_horizontally_does_not_teleport() {
        // When the horizontal leg fails (obstacle), fly_to must NOT teleport
        // past the blockage.
        let (executor, sender, state, log) = make_executor();
        log.goto_succeeds.store(false, Ordering::SeqCst);
        state.update_snapshot(crate::types::WorldSnapshot {
            self_player: SelfPlayer {
                position: BlockPos::new(0, 64, 0),
                gamemode: GameMode::Creative,
                ..Default::default()
            },
            ..Default::default()
        });
        let handle = spawn_executor(executor);

        let target = BlockPos::new(10, 70, 0);
        let result = send_and_await(&sender, BotCommand::FlyTo(target)).await;
        let br = result.expect("blocked fly_to is Ok(BotResult)");
        assert!(!br.success);
        let data = br.data.expect("data present");
        assert_eq!(data.get("reached"), Some(&serde_json::json!(false)));
        assert_eq!(data.get("reason"), Some(&serde_json::json!("obstacle")));

        drop(sender);
        handle.await.expect("executor should finish");

        // No /tp must have been issued past the obstacle.
        assert!(log.chat_calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_fly_to_not_creative_is_permission_denied() {
        // Act(Fly) reaches the executor without the MCP layer's creative
        // gate, so `handle_fly_to` must reject non-creative mode itself
        // instead of returning `success: true, reached: false`.
        let (executor, sender, state, log) = make_executor();
        state.update_snapshot(crate::types::WorldSnapshot {
            self_player: SelfPlayer {
                position: BlockPos::new(0, 64, 0),
                gamemode: GameMode::Survival,
                ..Default::default()
            },
            ..Default::default()
        });
        let handle = spawn_executor(executor);

        let target = BlockPos::new(10, 70, 0);
        let result = send_and_await(&sender, BotCommand::FlyTo(target)).await;
        assert!(
            matches!(result, Err(BotError::PermissionDenied(_))),
            "expected PermissionDenied, got: {result:?}"
        );

        drop(sender);
        handle.await.expect("executor should finish");

        // No movement or /tp must have been attempted.
        assert!(log.goto_calls.lock().unwrap().is_empty());
        assert!(log.chat_calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_fly_to_inner_goto_honours_fly_timeout() {
        // M-1: `fly_timeout_secs` must reach the pathfinder's
        // wait-for-completion loop (via `goto_with_deadline`), not just the
        // outer envelope. Configure a LONG fly timeout and a SHORT command
        // timeout; the recorded `goto_with_deadline` deadline must be the
        // fly-based value — flights needing > command_timeout_secs of path
        // search were previously killed at `command_timeout_secs`.
        let config = AppConfig {
            command_timeout_secs: 2,
            fly_timeout_secs: 8,
            ..AppConfig::default()
        };
        let state = Arc::new(SharedState::new(config));
        let (sender, receiver) = create_command_channel(16, Arc::clone(&state));
        state.set_online(true);
        state.update_snapshot(crate::types::WorldSnapshot {
            self_player: SelfPlayer {
                position: BlockPos::new(0, 64, 0),
                gamemode: GameMode::Creative,
                ..Default::default()
            },
            ..Default::default()
        });
        let mock = MockBotClient::new();
        let log = mock.log().clone();
        let executor = CommandExecutor::new(mock, Arc::clone(&state), receiver, None);
        let handle = spawn_executor(executor);

        let target = BlockPos::new(10, 70, 0);
        let result = send_and_await(&sender, BotCommand::FlyTo(target)).await;
        assert!(result.is_ok(), "fly_to should succeed: {result:?}");

        drop(sender);
        handle.await.expect("executor should finish");

        // The horizontal leg's goto_with_deadline must have received the fly
        // timeout (8 s), NOT the command timeout (2 s).
        let calls = log.goto_with_deadline_calls.lock().unwrap();
        assert_eq!(calls.len(), 1, "one horizontal-leg goto");
        let (pos, deadline) = calls[0];
        assert_eq!(
            pos,
            BlockPos::new(10, 64, 0),
            "horizontal leg keeps current Y"
        );
        assert_eq!(
            deadline.as_secs(),
            8,
            "inner goto must be bounded by fly_timeout_secs (8s), not command_timeout_secs (2s)"
        );
    }

    #[test]
    fn test_is_collectible_item_entity_exact_match_only() {
        assert!(is_collectible_item_entity("item"));
        assert!(is_collectible_item_entity("ITEM"));
        assert!(is_collectible_item_entity("item_entity"));
        assert!(!is_collectible_item_entity("item_frame"));
        assert!(!is_collectible_item_entity("item_display"));
        assert!(!is_collectible_item_entity("zombie"));
    }

    // ═══════════════════════════════════════════════════════════════
    // CollectItems tests (F6-2 — entities are now rebuilt from the live
    // ECS by SnapshotUpdater, so dropped items actually appear in the
    // snapshot and can be collected)
    // ═══════════════════════════════════════════════════════════════

    /// Snapshot with the player at (0, 64, 0) and the given entities.
    fn snapshot_with_entities(state: &SharedState, entities: Vec<EntityEntry>) {
        let snap = crate::types::WorldSnapshot {
            entities,
            self_player: SelfPlayer {
                uuid: "player-uuid".into(),
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
            ..Default::default()
        };
        state.update_snapshot(snap);
    }

    fn item_entity(pos: BlockPos) -> EntityEntry {
        EntityEntry {
            id: 11,
            uuid: "item-uuid".into(),
            entity_type: "item".into(),
            position: pos,
            display_name: None,
            health: None,
        }
    }

    #[tokio::test]
    async fn test_collect_items_finds_item_entities() {
        // A dropped item within radius must be visited (goto called with
        // the item's position) and counted in the result payload.
        let (executor, sender, state, log) = make_executor();
        snapshot_with_entities(&state, vec![item_entity(BlockPos::new(2, 64, 1))]);
        let handle = spawn_executor(executor);

        let result = send_and_await(&sender, BotCommand::CollectItems(5)).await;
        let br = result.expect("collect_items should succeed");
        assert!(br.success);
        assert!(br.message.contains("Visited 1"), "message: {}", br.message);
        let data = br.data.expect("data present");
        assert_eq!(data.get("visited"), Some(&serde_json::json!(1)));

        drop(sender);
        handle.await.expect("executor should finish");

        let goto_calls = log.goto_calls.lock().unwrap();
        assert_eq!(goto_calls.len(), 1);
        assert_eq!(goto_calls[0], BlockPos::new(2, 64, 1));
    }

    #[tokio::test]
    async fn test_collect_items_ignores_item_frames() {
        // Item frames contain "item" in their type but are not pickup-able
        // — the filter must exclude them, leaving nothing to collect.
        let (executor, sender, state, log) = make_executor();
        snapshot_with_entities(
            &state,
            vec![EntityEntry {
                id: 12,
                uuid: "frame-uuid".into(),
                entity_type: "item_frame".into(),
                position: BlockPos::new(1, 64, 0),
                display_name: None,
                health: None,
            }],
        );
        let handle = spawn_executor(executor);

        let result = send_and_await(&sender, BotCommand::CollectItems(5)).await;
        let br = result.expect("collect_items should succeed");
        assert!(br.success);
        assert_eq!(br.message, "No items to collect");
        let data = br.data.expect("data present");
        assert_eq!(data.get("visited"), Some(&serde_json::json!(0)));

        drop(sender);
        handle.await.expect("executor should finish");

        assert!(log.goto_calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_collect_items_ignores_items_outside_radius() {
        let (executor, sender, state, log) = make_executor();
        snapshot_with_entities(&state, vec![item_entity(BlockPos::new(50, 64, 50))]);
        let handle = spawn_executor(executor);

        let result = send_and_await(&sender, BotCommand::CollectItems(5)).await;
        let br = result.expect("collect_items should succeed");
        assert_eq!(br.message, "No items to collect");

        drop(sender);
        handle.await.expect("executor should finish");

        assert!(log.goto_calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_collect_items_partial_visits_reported() {
        // M-5: collect_items with several targets must not race the whole
        // command envelope (the raw `goto` loop could exceed it entirely, and
        // a success at the deadline was misreported as CommandTimeout). A
        // per-target budget — the remaining envelope budget split across the
        // remaining targets, floored at 2 s — stops the loop on the first
        // timeout and reports the partial visit count honestly: an Ok result,
        // not an Err, and not a fake "visited all".
        let config = AppConfig {
            command_timeout_secs: 2,
            ..AppConfig::default()
        };
        let state = Arc::new(SharedState::new(config));
        let (sender, receiver) = create_command_channel(16, Arc::clone(&state));
        state.set_online(true);
        let mock = MockBotClient::new();
        // First goto succeeds; the SECOND hangs → the per-target budget
        // times out mid-loop.
        mock.log.goto_hangs_after_n.store(2, Ordering::SeqCst);
        let executor = CommandExecutor::new(mock, Arc::clone(&state), receiver, None);
        snapshot_with_entities(
            &state,
            vec![
                item_entity(BlockPos::new(2, 64, 1)),
                item_entity(BlockPos::new(3, 64, 2)),
                item_entity(BlockPos::new(4, 64, 3)),
            ],
        );
        let handle = spawn_executor(executor);

        let result = send_and_await(&sender, BotCommand::CollectItems(5)).await;
        let br =
            result.expect("collect_items budget timeout returns Ok(BotResult), not an error (M-5)");
        assert!(
            !br.success,
            "budget exhausted before all targets → honest failure: {br:?}"
        );
        let data = br.data.expect("data present");
        assert_eq!(
            data["visited"], 1,
            "first target visited, then the per-target budget ran out"
        );
        assert!(br.message.contains("Visited 1"), "message: {}", br.message);
        assert!(
            br.message.contains("budget") || br.message.contains("skipped"),
            "message notes the skipped targets: {}",
            br.message
        );

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
        let info = found.expect("interpolated line scan must find the obstacle");
        assert_eq!(info.block_type, "stone");
        assert_eq!(info.position, obstacle);
    }

    #[test]
    fn test_find_obstacle_block_three_y_layers() {
        use std::collections::HashMap;
        // Obstacle at head height (y+1) on the direct line — the old scan
        // (current Y only) would miss it.
        let mut block_index = HashMap::new();
        block_index.insert(BlockPos::new(2, 65, 0), 0usize);
        let snapshot = crate::types::WorldSnapshot {
            blocks: vec![BlockEntry {
                position: BlockPos::new(2, 65, 0),
                block_type: "dirt".into(),
                block_state: None,
            }],
            block_index,
            ..Default::default()
        };

        let found =
            find_obstacle_block(&snapshot, BlockPos::new(0, 64, 0), BlockPos::new(4, 64, 0));
        let info = found.expect("y+1 layer scan must find the obstacle");
        assert_eq!(info.block_type, "dirt");
        assert_eq!(info.position, BlockPos::new(2, 65, 0));
    }

    #[test]
    fn test_find_obstacle_block_neighbourhood_fallback() {
        use std::collections::HashMap;
        // No solid block on the line, but one adjacent to `current` — the
        // 3×3×3 neighbourhood fallback must report it.
        let mut block_index = HashMap::new();
        block_index.insert(BlockPos::new(1, 64, 0), 0usize);
        let snapshot = crate::types::WorldSnapshot {
            blocks: vec![BlockEntry {
                position: BlockPos::new(1, 64, 0),
                block_type: "cobblestone".into(),
                block_state: None,
            }],
            block_index,
            ..Default::default()
        };

        let found =
            find_obstacle_block(&snapshot, BlockPos::new(0, 64, 0), BlockPos::new(0, 64, 0));
        let info = found.expect("neighbourhood fallback must find an obstacle");
        assert_eq!(info.block_type, "cobblestone");
        assert_eq!(info.position, BlockPos::new(1, 64, 0));
    }

    /// S-3 regression: the interpolation product `total_dx * i` overflows
    /// `i32` for long lines (coordinates are validated to ±30,000,000, so
    /// `total_dx` reaches 60,000,000 and the product exceeds `i32::MAX` at
    /// `i ≈ 36`). The old all-`i32` code panicked in debug builds — the
    /// executor task died mid-command and every later command surfaced
    /// `Offline("bot command responder dropped without reply")`. The scan now
    /// interpolates in `i64` and must find an obstacle well inside the step
    /// cap on a maximum-order line without overflowing.
    #[test]
    fn test_find_obstacle_block_long_line_no_i32_overflow() {
        use std::collections::HashMap;
        // 30,000,000-block line along +X. Obstacle at x=5,000 — step 5,000 of
        // 30,000,000, comfortably inside MAX_OBSTACLE_SCAN_STEPS. In the old
        // i32 arithmetic `total_dx * i` = 30,000,000 × 5,000 = 1.5e14
        // overflowed (panics in debug, wraps in release).
        let obstacle = BlockPos::new(5_000, 64, 0);
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

        let found = find_obstacle_block(
            &snapshot,
            BlockPos::new(0, 64, 0),
            BlockPos::new(30_000_000, 64, 0),
        );
        let info = found.expect("long-line scan must find the obstacle without overflowing");
        assert_eq!(info.block_type, "stone");
        assert_eq!(info.position, obstacle);
    }

    /// S-3 companion: a maximum-length negative-direction line with NO
    /// obstacle on the capped scan prefix must not panic or wrap — it falls
    /// through to the neighbourhood scan (also empty here → `None`).
    #[test]
    fn test_find_obstacle_block_max_distance_clear_line_no_panic() {
        use std::collections::HashMap;
        let snapshot = crate::types::WorldSnapshot {
            blocks: Vec::new(),
            block_index: HashMap::new(),
            ..Default::default()
        };

        let found = find_obstacle_block(
            &snapshot,
            BlockPos::new(30_000_000, 64, 30_000_000),
            BlockPos::new(-30_000_000, 64, -30_000_000),
        );
        assert!(found.is_none());
    }

    /// S-3 companion: proportional interpolation stays exact on a long
    /// diagonal — the obstacle sits on the scanned cells only because both
    /// axes interpolate proportionally (z advances at half the rate of x).
    #[test]
    fn test_find_obstacle_block_long_diagonal_interpolates() {
        use std::collections::HashMap;
        // Line (0,64,0) → (200_000,64,100_000): at step 8,000 (inside the
        // scan cap) the scan visits (8_000, 64, 4_000) exactly.
        let obstacle = BlockPos::new(8_000, 64, 4_000);
        let mut block_index = HashMap::new();
        block_index.insert(obstacle, 0usize);
        let snapshot = crate::types::WorldSnapshot {
            blocks: vec![BlockEntry {
                position: obstacle,
                block_type: "obsidian".into(),
                block_state: None,
            }],
            block_index,
            ..Default::default()
        };

        let found = find_obstacle_block(
            &snapshot,
            BlockPos::new(0, 64, 0),
            BlockPos::new(200_000, 64, 100_000),
        );
        let info = found.expect("diagonal interpolation must hit the midpoint obstacle");
        assert_eq!(info.position, obstacle);
    }

    // ═══════════════════════════════════════════════════════════════
    // Query tests
    // ═══════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_query_inventory() {
        let (executor, sender, _state, _log) = make_executor();
        let handle = spawn_executor(executor);

        let result = send_and_await(&sender, BotCommand::QueryInventory).await;
        assert!(result.is_ok());

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
            BotCommand::Act(ActAction::Attack { entity_id: 99 }, None),
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

    #[tokio::test]
    async fn test_act_wrapping_blocked_smart_move_reports_reason() {
        // F2-6 end-to-end: a blocked SmartMove sub-result (success == false)
        // must surface through `handle_act` as `ActResult.reason = Some(..)`
        // so the wrapping BotResult's success (derived from
        // `reason.is_none()`) is false — the `act` tool must not report
        // success for a move that never happened.
        let (executor, sender, _state, log) = make_executor();
        log.goto_succeeds.store(false, Ordering::SeqCst);
        let handle = spawn_executor(executor);

        let result = send_and_await(
            &sender,
            BotCommand::Act(
                ActAction::SmartMove {
                    target: BlockPos::new(5, 64, 5),
                },
                None,
            ),
        )
        .await;
        assert!(result.is_ok(), "expected Ok, got: {:?}", result);
        let br = result.unwrap();
        assert!(
            !br.success,
            "expected success == false for blocked smart_move via act"
        );

        let act_result: ActResult = serde_json::from_value(br.data.expect("act data present"))
            .expect("data is a serialized ActResult");
        assert!(
            act_result.reason.is_some(),
            "expected reason to be Some for blocked smart_move"
        );
        assert!(
            act_result
                .reason
                .as_deref()
                .unwrap_or_default()
                .contains("blocked"),
            "reason should carry the blocked message: {:?}",
            act_result.reason
        );

        drop(sender);
        handle.await.expect("executor should finish");
    }

    #[tokio::test]
    async fn test_act_perception_radius_trims_nearby_context() {
        // The act result must honour the per-call perception radius: with
        // Some(2) the stone block at (5,64,0) and the zombie at (3,64,1) are
        // outside the Chebyshev radius; with None the configured default
        // (block_perception_radius = 32) includes both.
        let (executor, sender, state, _log) = make_executor();
        make_populated_snapshot(&state);
        let handle = spawn_executor(executor);

        let trimmed = send_and_await(
            &sender,
            BotCommand::Act(
                ActAction::Move {
                    target: BlockPos::new(0, 64, 0),
                },
                Some(2),
            ),
        )
        .await
        .expect("act with radius should succeed");
        let small: ActResult = serde_json::from_value(trimmed.data.expect("data present"))
            .expect("data is a serialized ActResult");
        assert!(
            small.nearby_blocks.is_empty(),
            "radius 2 must exclude block at (5,64,0)"
        );
        assert!(
            small.nearby_entities.is_empty(),
            "radius 2 must exclude entity at (3,64,1)"
        );

        let full = send_and_await(
            &sender,
            BotCommand::Act(
                ActAction::Move {
                    target: BlockPos::new(0, 64, 0),
                },
                None,
            ),
        )
        .await
        .expect("act without radius should succeed");
        let wide: ActResult = serde_json::from_value(full.data.expect("data present"))
            .expect("data is a serialized ActResult");
        assert_eq!(
            wide.nearby_blocks.len(),
            1,
            "default radius must include block at (5,64,0)"
        );
        assert_eq!(
            wide.nearby_entities.len(),
            1,
            "default radius must include entity 42"
        );

        drop(sender);
        handle.await.expect("executor should finish");
    }

    #[tokio::test]
    async fn test_act_self_info_uses_live_position_after_move() {
        // Regression for the functional-test observation: `act(Move)` reported
        // the target but `self_info.position` was still the PRE-move position
        // from the throttled snapshot. The ActResult must prefer the bot's
        // live position (read at result build time) over the possibly-stale
        // snapshot.
        let (executor, sender, state, log) = make_executor();
        let mut snapshot = make_populated_snapshot_defaults();
        // Snapshot is stale: player still at (0,64,0).
        snapshot.self_player.position = BlockPos::new(0, 64, 0);
        snapshot.self_player.position_precise = None;
        state.update_snapshot(snapshot);
        // Live client position: the bot already arrived at (40.5, -60, -16.5).
        *log.live_position.lock().unwrap() = Some([40.5, -60.0, -16.5]);
        let handle = spawn_executor(executor);

        let result = send_and_await(
            &sender,
            BotCommand::Act(
                ActAction::Move {
                    target: BlockPos::new(40, -60, -16),
                },
                Some(0),
            ),
        )
        .await
        .expect("act should succeed");
        let act: ActResult = serde_json::from_value(result.data.expect("data present"))
            .expect("data is a serialized ActResult");
        assert_eq!(
            act.self_info.position,
            BlockPos::new(40, -60, -17),
            "self_info must reflect the live position (floor of 40.5, -60, -16.5), not the stale snapshot"
        );
        // Note: `position_precise` is `#[serde(skip)]` by design (W-1), so it
        // does not survive the JSON round-trip this test deserializes — the
        // integer `position` above is the observable contract.

        drop(sender);
        handle.await.expect("executor should finish");
    }

    #[tokio::test]
    async fn test_act_result_carries_live_position_without_snapshot_mutation() {
        // L-15: `handle_act` used to `Arc::make_mut` the whole snapshot to
        // override `self_player.position` — deep-cloning blocks + block_index
        // (potentially huge) on every act call because the Arc is shared
        // (the MCP/UI layers hold their own loads). The snapshot Arc must be
        // untouched, and the result must still carry the live position.
        let (executor, sender, state, log) = make_executor();
        let mut snapshot = make_populated_snapshot_defaults();
        snapshot.self_player.position = BlockPos::new(0, 64, 0);
        snapshot.self_player.position_precise = None;
        state.update_snapshot(snapshot);
        let snap_before = state.read_snapshot();
        // Live client position: the bot already arrived at (40.5, -60, -16.5).
        *log.live_position.lock().unwrap() = Some([40.5, -60.0, -16.5]);
        let handle = spawn_executor(executor);

        let result = send_and_await(
            &sender,
            BotCommand::Act(
                ActAction::Move {
                    target: BlockPos::new(40, -60, -16),
                },
                Some(0),
            ),
        )
        .await
        .expect("act should succeed");
        let act: ActResult = serde_json::from_value(result.data.expect("data present"))
            .expect("data is a serialized ActResult");
        assert_eq!(
            act.self_info.position,
            BlockPos::new(40, -60, -17),
            "self_info must carry the live position, not the stale snapshot"
        );

        drop(sender);
        handle.await.expect("executor should finish");

        let snap_after = state.read_snapshot();
        assert!(
            Arc::ptr_eq(&snap_before, &snap_after),
            "handle_act must not replace the snapshot Arc (L-15): Arc::make_mut \
             would deep-clone the whole snapshot (blocks + block_index) on \
             every act call"
        );
        // The live-position override must NOT leak into the shared snapshot —
        // other readers (MCP tools, the UI) must keep seeing the throttled
        // world state, not a per-call act override.
        assert_eq!(
            snap_after.self_player.position,
            BlockPos::new(0, 64, 0),
            "the stored snapshot must be unmutated by handle_act"
        );
        assert_eq!(
            snap_after.self_player.position_precise, None,
            "the stored snapshot's precise position must stay untouched"
        );
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
