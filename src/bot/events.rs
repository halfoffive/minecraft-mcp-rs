//! Event processing from the Minecraft client (chat, move, damage, etc.).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use azalea::ecs::component::Component;
use azalea::pathfinder::PathfinderClientExt;
use azalea::prelude::AppExit;
use azalea::protocol::packets::game::ClientboundGamePacket;
use azalea::{Client, Event};
use tracing::{info, trace, warn};

use super::commands::{CommandExecutor, RealBotClient};
use super::snapshot_updater::SnapshotUpdater;
use crate::channel::{BotCommandSender, ReceiverLease, ReceiverSlot};
use crate::snapshot::DirtyTracker;
use crate::state::SharedState;
use crate::types::BlockPos;

// ---------------------------------------------------------------------------
// Dependency injection — set before ClientBuilder::start()
// ---------------------------------------------------------------------------

/// Pre-initialized shared state to inject into [`BotState`] before the bot
/// starts. Set by [`crate::bot::connection::ConnectionManager::connect`] and
/// cleared on disconnect so a subsequent connection (or a test) sees a clean
/// slot.
///
/// If not set, [`BotState::default`] falls back to creating an isolated
/// [`SharedState`] (useful for unit tests).
pub(crate) static INJECTED_SHARED_STATE: Mutex<Option<Arc<SharedState>>> = Mutex::new(None);

/// Pre-initialized command receiver slot to inject into [`BotState`].
///
/// The receiver is stored behind `Mutex<Option<_>>` so the event handler can
/// [`ReceiverLease::take`] it on `Event::Spawn` and the command executor can
/// run with it; when the executor is aborted the lease returns the receiver
/// to this slot, allowing a future `Spawn` (reconnect) to re-acquire it.
/// Set by [`crate::bot::connection::ConnectionManager::connect`] and cleared
/// on disconnect.
pub(crate) static INJECTED_COMMAND_RECEIVER: Mutex<Option<ReceiverSlot>> = Mutex::new(None);

/// Pre-initialized egui context to inject into [`BotState`] (optional).
/// Set by [`crate::bot::connection::ConnectionManager::connect`] and cleared
/// on disconnect.
pub(crate) static INJECTED_EGUI_CTX: Mutex<Option<egui::Context>> = Mutex::new(None);

/// Pre-initialized command sender to inject into [`BotState`] (optional).
///
/// Used to give the command executor a way to issue sub-commands (e.g.
/// `Act::Mine` delegating to `CompoundOpExecutor` which sends `MoveTo` /
/// `BreakBlock` back through the channel). `None` in unit tests where the
/// executor's `handle_act` falls back to fire-and-forget behaviour.
/// Set by [`crate::bot::connection::ConnectionManager::connect`] and cleared
/// on disconnect.
pub(crate) static INJECTED_COMMAND_SENDER: Mutex<Option<BotCommandSender>> = Mutex::new(None);

/// Pre-initialized snapshot interval to inject into [`BotState`].
///
/// Uses [`AtomicU64`] (rather than [`OnceLock`]) so the value can be
/// refreshed on each reconnect — `OnceLock::set` only succeeds once, which
/// silently dropped updates to `snapshot_interval_ms` on subsequent
/// connection attempts. A value of `0` means "not set"; [`BotState::default`]
/// falls back to `500` in that case.
pub(crate) static INJECTED_SNAPSHOT_INTERVAL_MS: AtomicU64 = AtomicU64::new(0);

// ---------------------------------------------------------------------------
// BotState
// ---------------------------------------------------------------------------

/// State carried by the azalea event handler.
///
/// Must implement [`Clone`] + [`Component`] + [`Default`] because azalea
/// requires the state to be an ECS component and clones it for each handler
/// invocation.
#[derive(Clone, Component)]
pub struct BotState {
    /// Shared application state — updated by the handler, read by MCP and UI.
    pub shared_state: Arc<SharedState>,
    /// Slot holding the command receiver, leased out to the command executor
    /// on `Event::Spawn`. See `ReceiverLease`.
    pub command_receiver: ReceiverSlot,
    /// Handle to the running command executor task (if any). Aborted on
    /// disconnect so the stale azalea `Client` is never used after the
    /// connection drops; the leased receiver is returned to
    /// [`BotState::command_receiver`] by the `ReceiverLease` drop guard.
    pub executor_handle: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
    /// Tick-local snapshot tasks tracked so their handles can be reclaimed
    /// instead of leaking unboundedly.
    pub tick_tasks: Arc<Mutex<tokio::task::JoinSet<()>>>,
    /// Optional egui context for requesting UI repaints.
    pub egui_ctx: Option<egui::Context>,
    /// Tracks which blocks/chunks changed since the last snapshot.
    pub dirty_tracker: Arc<Mutex<DirtyTracker>>,
    /// Last time a snapshot was written to [`SharedState`].
    pub last_snapshot_time: Arc<Mutex<Instant>>,
    /// Minimum milliseconds between snapshot updates.
    pub snapshot_interval_ms: u64,
    /// Optional command sender for issuing sub-commands (e.g. `Act::Mine`
    /// delegating to `CompoundOpExecutor`). `None` in unit tests; set from
    /// `INJECTED_COMMAND_SENDER` in production.
    pub command_sender: Option<BotCommandSender>,
}

impl Default for BotState {
    fn default() -> Self {
        let shared_state = INJECTED_SHARED_STATE
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .unwrap_or_else(|| Arc::new(SharedState::new(crate::config::AppConfig::default())));

        let command_receiver = INJECTED_COMMAND_RECEIVER
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .unwrap_or_else(|| {
                let (_, receiver) = crate::channel::create_command_channel(
                    1,
                    Arc::new(SharedState::new(crate::config::AppConfig::default())),
                );
                Arc::new(Mutex::new(Some(receiver)))
            });

        let egui_ctx = INJECTED_EGUI_CTX
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();

        let command_sender = INJECTED_COMMAND_SENDER
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();

        let snapshot_interval_ms = {
            let v = INJECTED_SNAPSHOT_INTERVAL_MS.load(Ordering::Relaxed);
            // A value of 0 means "not injected yet" — fall back to the
            // default 500 ms so unit tests and any pre-inject path still
            // get a sane throttle interval.
            if v == 0 { 500 } else { v }
        };

        Self {
            shared_state,
            command_receiver,
            executor_handle: Arc::new(Mutex::new(None)),
            tick_tasks: Arc::new(Mutex::new(tokio::task::JoinSet::new())),
            egui_ctx,
            dirty_tracker: Arc::new(Mutex::new(DirtyTracker::new())),
            last_snapshot_time: Arc::new(Mutex::new(Instant::now() - Duration::from_secs(3600))),
            snapshot_interval_ms,
            command_sender,
        }
    }
}

// ---------------------------------------------------------------------------
// handle_event
// ---------------------------------------------------------------------------

/// Main azalea event handler.
///
/// This is a function pointer (no closures) so azalea can call it from the ECS.
/// Heavy work is offloaded via [`tokio::task::spawn_local`] where appropriate.
pub async fn handle_event(bot: Client, event: Event, state: BotState) -> eyre::Result<()> {
    match event {
        Event::Spawn => {
            handle_spawn(bot, &state).await;
        }
        Event::Disconnect(_) => {
            handle_disconnect(bot, &state).await;
        }
        Event::Tick => {
            handle_tick(bot, state).await;
        }
        Event::Chat(chat_packet) => {
            handle_chat(&state, chat_packet);
        }
        Event::Death(_) => {
            handle_death(&state);
        }
        Event::ReceiveChunk(chunk_pos) => {
            handle_receive_chunk(&state, chunk_pos);
        }
        Event::Packet(packet) => {
            handle_packet_block_updates(&state, &packet);
        }
        // NOTE: `AddPlayer` / `RemovePlayer` / `UpdatePlayer` (and every
        // other event) are intentionally ignored here. The snapshot's
        // entity list is rebuilt from the live ECS on every snapshot tick
        // by `SnapshotUpdater` (see `snapshot_updater::collect_entities`),
        // so player join/leave/update events no longer need to maintain
        // `WorldSnapshot::entities` incrementally.
        _ => {}
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Event helpers
// ---------------------------------------------------------------------------

/// Start the command executor and store the bot's ECS handle.
///
/// Made `async` so it can call `ReceiverLease::take_with_retry`, which
/// needs to `await` between polls. The `handle_event` function calls
/// this with `.await` so the runtime can drive the retry loop.
async fn handle_spawn(bot: Client, state: &BotState) {
    // NOTE: set_online(true) is intentionally deferred until after the
    // command executor is successfully started (see below).  Reporting
    // "online" before the executor is ready would cause MCP clients to
    // send commands that receive Offline errors.

    // Store the ECS handle so request_disconnect can trigger shutdown by
    // writing AppExit::Success to the ECS World (same pattern as
    // handle_disconnect below). Without this, the Disconnect button can
    // only cancel the backoff sleep — it cannot interrupt a running
    // ClientBuilder::start().
    let ecs = bot.ecs.clone();
    state
        .shared_state
        .set_bot_ecs(crate::state::BotEcsHandle::new(move || {
            ecs.lock().write_message(AppExit::Success);
        }));

    // Abort any previous command executor (e.g. left over from a prior
    // connection that dropped without firing Disconnect). Aborting drops the
    // ReceiverLease, which returns the receiver to the slot below.
    //
    // Take the handle out of the mutex first and drop the lock before
    // calling `abort()` — `JoinHandle::abort` may park/schedule and must
    // not be called while holding the `executor_handle` mutex (the aborted
    // task's cleanup path could otherwise try to re-acquire it).
    let handle_to_abort = {
        let mut handle_guard = state
            .executor_handle
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        handle_guard.take()
    };
    if let Some(handle) = handle_to_abort {
        handle.abort();
        info!("aborted previous command executor before starting a new one");
    }

    // Lease the command receiver and start a new executor driving it.
    match ReceiverLease::take_with_retry(&state.command_receiver).await {
        Some(lease) => {
            let shared_state = Arc::clone(&state.shared_state);
            // Clone the sender outside the async move closure so the
            // closure doesn't borrow `state` (which lives longer than
            // the closure but the borrow checker doesn't know that
            // here — see E0521). The sender is needed by both
            // `RealBotClient` (for `sender.timeout()` lookups in
            // `goto` so the pathfinder timeout stays in lock-step with
            // the command-channel timeout) and `CommandExecutor` (for
            // recursive compound-op dispatch). `BotState::command_sender`
            // is `Option<BotCommandSender>` (None in unit tests where
            // there is no `McpBotServer` injecting a sender); in
            // production `connection.rs` always sets it to `Some`,
            // and `Event::Spawn` only fires after that injection, so
            // the unwrap is safe at runtime. The `Option<...>` is
            // passed through to `CommandExecutor` unchanged.
            let command_sender_opt = state.command_sender.clone();
            let Some(command_sender) = command_sender_opt.clone() else {
                warn!(
                    "Spawn fired but command_sender was not injected — \
                     skipping executor start (this indicates azalea \
                     fired Event::Spawn before ConnectionManager completed \
                     dependency injection)"
                );
                state.shared_state.clear_bot_ecs();
                request_repaint(state);
                trace!("bot spawned without executor — not marking online");
                return;
            };
            let client = RealBotClient::new(bot, Arc::clone(&shared_state), command_sender);
            let handle = tokio::task::spawn_local(async move {
                let mut executor =
                    CommandExecutor::new_for_lease(client, shared_state, command_sender_opt);
                executor.run_with_lease(lease).await;
            });
            *state
                .executor_handle
                .lock()
                .unwrap_or_else(|e| e.into_inner()) = Some(handle);
            // Mark the bot online only after the executor is running so
            // MCP clients that observe is_online()==true can immediately
            // send commands without hitting Offline errors.
            state.shared_state.set_online(true);
            // Latch "this session came online" for the connect loop
            // (audit F6-1 fix). `handle_disconnect` clears `is_online()`
            // BEFORE `ClientBuilder::start()` returns, so the loop cannot
            // use `is_online()` to learn that this session ever connected
            // — it would always see `false` and skip the exponential
            // backoff branch. The latch survives the disconnect and is
            // consumed by `SharedState::take_session_was_online()` in the
            // loop, making reconnect-with-backoff reachable after real
            // disconnects.
            state.shared_state.mark_session_online();
            state.shared_state.set_connected_since(Some(Instant::now()));
            info!("command executor started");
        }
        None => {
            warn!(
                "Spawn fired but no command receiver was available after \
                 100ms retry window — executor not started (this is expected \
                 if the previous executor is still shutting down)"
            );
            state.shared_state.clear_bot_ecs();
        }
    }

    request_repaint(state);
    trace!("handle_spawn completed");
}

/// Abort any in-flight tick snapshot tasks and drop their handles.
///
/// The `JoinSet` is replaced with an empty one so the next connection starts
/// fresh. This helper is split out so it can be exercised in unit tests.
async fn abort_and_clear_tick_tasks(tick_tasks: &Mutex<tokio::task::JoinSet<()>>) {
    let mut set = std::mem::take(&mut *tick_tasks.lock().unwrap_or_else(|e| e.into_inner()));
    if !set.is_empty() {
        set.abort_all();
        while set.join_next().await.is_some() {}
    }
}

async fn handle_disconnect(bot: Client, state: &BotState) {
    state.shared_state.set_online(false);
    state.shared_state.set_connected_since(None);

    // Drop the cached world-view render so the UI preview panel (and a
    // later `get_world_view` call) does not keep showing a frame from the
    // previous connection after the bot goes offline.
    state.shared_state.clear_world_view_cache();

    // Clear the ECS handle — the bot is already disconnecting, so
    // request_disconnect no longer needs to write AppExit::Success.
    state.shared_state.clear_bot_ecs();

    // Abort the command executor so it can't use the now-stale azalea Client
    // (which would panic when touching the ECS after disconnect). The
    // ReceiverLease guard drops and returns the receiver to the slot, ready
    // for the next Spawn.
    let handle_to_abort = {
        let mut handle_guard = state
            .executor_handle
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        handle_guard.take()
    };
    let aborted = if let Some(handle) = handle_to_abort {
        handle.abort();
        // Yield briefly to allow the aborted task to return the receiver
        // via ReceiverLease::Drop before a fast reconnect fires Spawn.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        true
    } else {
        false
    };
    if aborted {
        info!("aborted command executor on disconnect");
    }

    // Abort in-flight tick snapshot tasks and reclaim their handles. This
    // prevents the per-tick `spawn_local` handle list from growing forever
    // across reconnects.
    abort_and_clear_tick_tasks(&state.tick_tasks).await;

    // Clear the injected dependencies so the next connection (or a test in
    // the same process) starts from a clean slot. With `OnceLock` the first
    // `set` would silently win forever, leaking state across reconnects and
    // between tests; `Mutex<Option<_>>` lets us reset here.
    *INJECTED_SHARED_STATE
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = None;
    *INJECTED_COMMAND_RECEIVER
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = None;
    *INJECTED_EGUI_CTX.lock().unwrap_or_else(|e| e.into_inner()) = None;
    *INJECTED_COMMAND_SENDER
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = None;
    INJECTED_SNAPSHOT_INTERVAL_MS.store(0, Ordering::Relaxed);

    // Tell azalea to end the client so ClientBuilder::start returns and the
    // connection loop can retry. Without this the bot thread may hang waiting
    // for an ECS that's already shutting down. In azalea 0.15.1 there is no
    // `bot.exit()` method, so we send an `AppExit` message through the ECS
    // World to trigger a clean shutdown.
    bot.ecs.lock().write_message(AppExit::Success);

    request_repaint(state);
    trace!("bot disconnected, set online=false");
}

async fn handle_tick(bot: Client, state: BotState) {
    // Wake any waiter in `RealBotClient::goto` when the pathfinder has
    // reached its target. `notify_waiters` is cheap when no one is waiting.
    if bot.is_goto_target_reached() {
        state.shared_state.notify_goto_reached();
    }

    // Build a SnapshotUpdater from the BotState's shared fields and delegate
    // the throttle check + snapshot build to it. This avoids duplicating the
    // snapshot logic that already lives in snapshot_updater.rs.
    let updater = SnapshotUpdater::new(
        Arc::clone(&state.shared_state),
        Arc::clone(&state.dirty_tracker),
        Arc::clone(&state.last_snapshot_time),
        state.snapshot_interval_ms,
    );
    let egui_ctx = state.egui_ctx.clone();

    // Throttle check BEFORE spawning the build task: azalea fires ~20 ticks
    // per second against a 500 ms snapshot interval, so a post-spawn check
    // would create (and immediately throw away) a task on every one of the
    // ~18 throttled ticks.
    if !updater.check_and_update_timer() {
        return;
    }

    // Reclaim any snapshot tasks that have already finished before adding a
    // new one. This prevents unbounded growth of `spawn_local` handles.
    let mut tick_tasks = state.tick_tasks.lock().unwrap_or_else(|e| e.into_inner());
    while let Some(res) = tick_tasks.try_join_next() {
        if let Err(e) = res {
            warn!("tick snapshot task finished with error: {}", e);
        }
    }

    tick_tasks.spawn_local(async move {
        if updater.build_and_store(&bot).await
            && let Some(ctx) = &egui_ctx
        {
            ctx.request_repaint();
        }
    });
}

fn handle_chat(state: &BotState, chat_packet: azalea::chat::ChatPacket) {
    let (sender, message) = chat_packet.split_sender_and_content();
    let sender = sender.unwrap_or_else(|| "System".to_string());
    state.shared_state.add_chat_message(sender, message);
    trace!("chat message stored");
}

fn handle_death(state: &BotState) {
    state
        .shared_state
        .modify_snapshot(|s| s.self_player.health = 0.0);
    request_repaint(state);
    trace!("bot died, set health=0");
}

// NOTE: the former `handle_add_player` / `handle_remove_player` /
// `handle_update_player` handlers were removed (audit F6-2). They seeded
// `WorldSnapshot::entities` from tab-list events only — players only,
// frozen at their join position — so `collect_items` and
// `get_nearby_entities` never saw dropped items or mobs. Entities are now
// rebuilt from the live ECS on every snapshot tick by `SnapshotUpdater`
// (see `snapshot_updater::collect_entities`).

fn handle_receive_chunk(state: &BotState, chunk_pos: azalea::core::position::ChunkPos) {
    let mut tracker = state
        .dirty_tracker
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    tracker.mark_chunk_dirty((chunk_pos.x, chunk_pos.z));
    trace!("chunk dirty marked: ({}, {})", chunk_pos.x, chunk_pos.z);
}

/// Inspect a server packet for block-change notifications and mark the
/// affected positions dirty in the [`DirtyTracker`].
///
/// Only `BlockUpdate` (single block) and `SectionBlocksUpdate` (batch within
/// one chunk section) are handled; all other packet variants are ignored.
/// The world-block positions are converted from azalea's `BlockPos` /
/// `ChunkSectionPos + ChunkSectionBlockPos` into the crate's [`BlockPos`]
/// before marking.
fn handle_packet_block_updates(state: &BotState, packet: &ClientboundGamePacket) {
    match packet {
        ClientboundGamePacket::BlockUpdate(data) => {
            let pos = BlockPos::new(data.pos.x, data.pos.y, data.pos.z);
            let mut tracker = state
                .dirty_tracker
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            tracker.mark_block_dirty(pos);
            trace!("block dirty marked: {}", pos);
        }
        ClientboundGamePacket::SectionBlocksUpdate(data) => {
            let mut tracker = state
                .dirty_tracker
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            for entry in &data.states {
                // ChunkSectionPos + ChunkSectionBlockPos yields azalea's
                // world-space BlockPos (see azalea_core::position impls).
                let world_pos = data.section_pos + entry.pos;
                let pos = BlockPos::new(world_pos.x, world_pos.y, world_pos.z);
                tracker.mark_block_dirty(pos);
            }
            trace!(
                "section blocks update: {} blocks marked dirty",
                data.states.len()
            );
        }
        _ => {}
    }
}

fn request_repaint(state: &BotState) {
    if let Some(ctx) = &state.egui_ctx {
        ctx.request_repaint();
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- BotState construction -----------------------------------------------

    #[test]
    fn test_bot_state_default() {
        let state = BotState::default();
        assert!(!state.shared_state.is_online());
        assert_eq!(state.snapshot_interval_ms, 500);
        assert!(state.egui_ctx.is_none());
    }

    #[test]
    fn test_bot_state_clone_shares_arc() {
        let state = BotState::default();
        let cloned = state.clone();
        assert!(Arc::ptr_eq(&state.shared_state, &cloned.shared_state));
        assert!(Arc::ptr_eq(&state.dirty_tracker, &cloned.dirty_tracker));
        assert!(Arc::ptr_eq(&state.tick_tasks, &cloned.tick_tasks));
    }

    #[test]
    fn test_bot_state_default_has_tick_tasks() {
        let state = BotState::default();
        let tick_tasks = state.tick_tasks.lock().unwrap();
        assert!(tick_tasks.is_empty());
    }

    // -- Event helpers (no Client needed) ------------------------------------

    // NOTE: `handle_spawn` and `handle_disconnect` now require an azalea
    // `Client` (they start/stop the command executor and call `bot.exit()`),
    // so they cannot be exercised in isolation here. Their online-flag
    // behaviour is covered by the `SharedState` tests in `state.rs`, and the
    // executor wiring is covered by `bot::commands` tests.

    #[test]
    fn test_death_sets_health_to_zero() {
        let state = BotState::default();
        handle_death(&state);
        let snapshot = state.shared_state.read_snapshot();
        assert_eq!(snapshot.self_player.health, 0.0);
    }

    #[test]
    fn test_receive_chunk_marks_dirty() {
        let state = BotState::default();
        let chunk_pos = azalea::core::position::ChunkPos::new(3, -7);
        handle_receive_chunk(&state, chunk_pos);
        let tracker = state.dirty_tracker.lock().unwrap();
        assert!(!tracker.is_empty());
    }

    // -- Chat handling -------------------------------------------------------

    #[test]
    fn test_chat_system_message() {
        let state = BotState::default();
        let chat = azalea::chat::ChatPacket::new("Hello world");
        handle_chat(&state, chat);
        let messages = state.shared_state.get_chat_messages();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].0, "System");
        assert_eq!(messages[0].1, "Hello world");
    }

    // NOTE: the former player-list tests (`test_add_player_updates_snapshot`,
    // `test_remove_player_updates_snapshot`, `test_update_player_updates_snapshot`)
    // were removed together with their handlers — entities are now rebuilt
    // from the live ECS by `SnapshotUpdater`, covered by the
    // `snapshot_updater::tests::test_collect_entities_*` tests.

    // -- Throttle logic ------------------------------------------------------

    #[test]
    fn test_tick_throttle_skips_fast_updates() {
        let state = BotState::default();
        state.shared_state.set_online(true);

        // Manually set last snapshot time to now.
        *state.last_snapshot_time.lock().unwrap() = Instant::now();

        // Should not update because interval hasn't passed.
        let should_update = {
            let last = state.last_snapshot_time.lock().unwrap();
            last.elapsed() >= Duration::from_millis(state.snapshot_interval_ms)
        };
        assert!(!should_update);
    }

    #[test]
    fn test_tick_throttle_allows_slow_updates() {
        let state = BotState::default();
        state.shared_state.set_online(true);

        // Set last snapshot time far in the past.
        *state.last_snapshot_time.lock().unwrap() = Instant::now() - Duration::from_secs(10);

        let should_update = {
            let last = state.last_snapshot_time.lock().unwrap();
            last.elapsed() >= Duration::from_millis(state.snapshot_interval_ms)
        };
        assert!(should_update);
    }

    // -- Tick task lifecycle -------------------------------------------------

    #[tokio::test(flavor = "current_thread")]
    async fn test_tick_tasks_drain_completed() {
        let state = BotState::default();
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                // Simulate a few prior tick tasks that finish immediately.
                {
                    let mut set = state.tick_tasks.lock().unwrap_or_else(|e| e.into_inner());
                    set.spawn_local(async {});
                    set.spawn_local(async {});
                    assert_eq!(set.len(), 2);
                }

                // Let the LocalSet run the spawned tasks to completion before
                // draining. `try_join_next` only reclaims tasks that have
                // already finished.
                tokio::task::yield_now().await;

                // Drain completed tasks the same way handle_tick does.
                {
                    let mut set = state.tick_tasks.lock().unwrap_or_else(|e| e.into_inner());
                    while let Some(res) = set.try_join_next() {
                        if let Err(e) = res {
                            warn!("test tick task error: {}", e);
                        }
                    }
                    assert!(set.is_empty());
                }
            })
            .await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_disconnect_aborts_tick_tasks() {
        let state = BotState::default();
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                // Spawn a tick task that would run forever if not aborted.
                {
                    let mut set = state.tick_tasks.lock().unwrap_or_else(|e| e.into_inner());
                    set.spawn_local(async {
                        loop {
                            tokio::task::yield_now().await;
                        }
                    });
                    assert_eq!(set.len(), 1);
                }

                abort_and_clear_tick_tasks(&state.tick_tasks).await;

                let set = state.tick_tasks.lock().unwrap_or_else(|e| e.into_inner());
                assert!(set.is_empty());
            })
            .await;
    }

    // -- Utility -------------------------------------------------------------
    //
    // azalea_gamemode_to_ours and to_snake_case tests were removed; both
    // functions now live exclusively in snapshot_updater.rs where they are
    // tested.
}
