//! Shared application state accessible across bot, MCP, and UI layers.
//!
//! [`SharedState`] is the central hub: the bot engine writes to it,
//! the MCP server reads from it, and the egui UI reads from it.
//! Thread safety is critical, so we use:
//!
//! - [`arc_swap::ArcSwap`] for the world snapshot — lock-free reads
//! - [`std::sync::RwLock`] for config and run stats — short locks, safe on UI thread
//! - [`std::sync::atomic::AtomicBool`] for the online flag — lock-free
//! - [`std::sync::Mutex`] for the optional [`ContainerHandle`] — azalea auto-closes on Drop

use arc_swap::ArcSwap;
use azalea::container::ContainerHandle;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock, RwLockReadGuard};
use tokio_util::sync::CancellationToken;

use crate::config::{AppConfig, RunStats};
use crate::types::WorldSnapshot;

// ---------------------------------------------------------------------------
// BotEcsHandle
// ---------------------------------------------------------------------------

/// Handle to the bot's ECS [`World`](bevy_ecs::world::World), used to trigger
/// [`ClientBuilder::start`](azalea::ClientBuilder::start) to return by writing
/// [`AppExit::Success`](azalea::prelude::AppExit) to the ECS.
///
/// The underlying `bot.ecs` field on azalea's [`Client`](azalea::Client) has
/// type `Arc<parking_lot::Mutex<World>>`. Because `parking_lot` is not a
/// direct dependency of this crate, that type cannot be named in a field
/// signature. Instead, [`BotEcsHandle`] stores a closure that captures
/// `bot.ecs.clone()` and invokes `ecs.lock().write_message(AppExit::Success)`
/// when called — the same pattern used by the `Event::Disconnect` handler in
/// `bot/events.rs`.
///
/// The closure is `Send + Sync` so the handle can be shared across threads
/// via [`SharedState`]. Cloning the handle is cheap (it clones an [`Arc`]).
#[derive(Clone)]
pub struct BotEcsHandle(Arc<dyn Fn() + Send + Sync>);

impl BotEcsHandle {
    /// Create a new handle from a closure that writes `AppExit::Success` to
    /// the bot's ECS World.
    ///
    /// In practice the closure captures `bot.ecs.clone()` (an
    /// `Arc<parking_lot::Mutex<World>>`) and calls
    /// `ecs.lock().write_message(AppExit::Success)`.
    pub fn new(write_app_exit: impl Fn() + Send + Sync + 'static) -> Self {
        Self(Arc::new(write_app_exit))
    }

    /// Invoke the stored closure, writing `AppExit::Success` to the ECS World.
    ///
    /// This causes `ClientBuilder::start()` to return, allowing the reconnect
    /// loop to exit or retry.
    pub fn write_app_exit(&self) {
        (self.0)();
    }
}

impl std::fmt::Debug for BotEcsHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BotEcsHandle").finish_non_exhaustive()
    }
}

// ---------------------------------------------------------------------------
// SharedState
// ---------------------------------------------------------------------------

/// Central thread-safe state shared by bot, MCP, and UI threads.
#[derive(Debug)]
pub struct SharedState {
    /// Lock-free world snapshot — replaced atomically by the bot engine.
    world_snapshot: ArcSwap<WorldSnapshot>,
    /// User settings — read by UI, written by UI settings panel.
    config: RwLock<AppConfig>,
    /// Command counters — updated by bot, displayed by UI.
    run_stats: RwLock<RunStats>,
    /// Whether the bot is currently connected to the server.
    bot_online: AtomicBool,
    /// Whether a bot connection attempt is in progress (guards against
    /// double-spawn when the user clicks Connect while the previous
    /// connection is still being established).
    bot_connecting: AtomicBool,
    /// Set by the Disconnect button to tell the reconnect loop to stop
    /// retrying. Cleared on the next Connect click.
    disconnect_requested: AtomicBool,
    /// Handle to the currently open container (if any).
    ///
    /// Stored behind `Mutex<Option<_>>` because [`ContainerHandle`] auto-closes
    /// on [`Drop`], so we must ensure only one owner exists at a time.
    container_handle: Mutex<Option<ContainerHandle>>,
    /// Last 10 chat messages received from the server.
    ///
    /// Each entry is `(sender, message)`. Stored behind a `Mutex` because the
    /// bot event handler writes to it from azalea's ECS thread.
    chat_messages: Mutex<VecDeque<(String, String)>>,
    /// Last error message reported by the bot/MCP layer, if any.
    ///
    /// Stored behind a `Mutex` because writers (bot event handlers, MCP
    /// tools) run on different threads than the reader (UI).
    last_error: Mutex<Option<String>>,
    /// Cancellation token used to interrupt the reconnect backoff sleep
    /// when the user requests a disconnect. Stored behind a `Mutex` so it
    /// can be replaced with a fresh token on each new connection attempt
    /// (see [`reset_cancel_token`](Self::reset_cancel_token)).
    cancel_token: Mutex<CancellationToken>,
    /// Handle to the bot's ECS World, set on `Event::Spawn` and cleared on
    /// `Event::Disconnect`. When [`request_disconnect`](Self::request_disconnect)
    /// is called, the handle's [`BotEcsHandle::write_app_exit`] is invoked,
    /// which writes `AppExit::Success` to the ECS and causes
    /// `ClientBuilder::start()` to return — the same pattern used by the
    /// `Event::Disconnect` handler in `bot/events.rs`. This is what actually
    /// closes a live TCP connection (cancelling the backoff sleep alone
    /// cannot interrupt a running `ClientBuilder::start()`).
    bot_ecs: Mutex<Option<BotEcsHandle>>,
}

impl SharedState {
    /// Create a new [`SharedState`] with the given config.
    ///
    /// The world snapshot starts empty, the bot is offline, and no container
    /// is open.
    pub fn new(config: AppConfig) -> Self {
        let empty_snapshot = WorldSnapshot {
            blocks: vec![],
            entities: vec![],
            self_player: crate::types::SelfPlayer {
                uuid: String::new(),
                username: String::new(),
                position: crate::types::BlockPos::new(0, 0, 0),
                health: 0.0,
                hunger: 0,
                gamemode: crate::types::GameMode::Survival,
                held_item_slot: 0,
                inventory: Vec::new(),
            },
            timestamp: 0,
            chunk_summary: vec![],
            commands_enabled: None,
        };

        Self {
            world_snapshot: ArcSwap::from_pointee(empty_snapshot),
            config: RwLock::new(config),
            run_stats: RwLock::new(RunStats::default()),
            bot_online: AtomicBool::new(false),
            bot_connecting: AtomicBool::new(false),
            disconnect_requested: AtomicBool::new(false),
            container_handle: Mutex::new(None),
            chat_messages: Mutex::new(VecDeque::new()),
            last_error: Mutex::new(None),
            cancel_token: Mutex::new(CancellationToken::new()),
            bot_ecs: Mutex::new(None),
        }
    }

    /// Atomically replace the world snapshot.
    ///
    /// Writers (bot engine) call this; readers (MCP, UI) see the new
    /// snapshot on their next [`load`](ArcSwap::load) without blocking.
    pub fn update_snapshot(&self, new: WorldSnapshot) {
        self.world_snapshot.store(Arc::new(new));
    }

    /// Lock-free read of the current world snapshot.
    ///
    /// Returns an [`Arc`] so the caller can hold the snapshot indefinitely
    /// without blocking subsequent updates.
    pub fn read_snapshot(&self) -> Arc<WorldSnapshot> {
        self.world_snapshot.load_full()
    }

    /// Set the bot online status atomically.
    pub fn set_online(&self, online: bool) {
        self.bot_online.store(online, Ordering::SeqCst);
    }

    /// Read the bot online status atomically.
    pub fn is_online(&self) -> bool {
        self.bot_online.load(Ordering::SeqCst)
    }

    /// Try to enter the "connecting" state. Returns `true` if the caller is
    /// the first to claim it (and should proceed to spawn the connection
    /// thread), `false` if another connection attempt is already in progress.
    ///
    /// The caller must call [`clear_connecting`](Self::clear_connecting) when
    /// the connection attempt finishes (success or failure) so future Connect
    /// clicks are accepted.
    pub fn try_begin_connecting(&self) -> bool {
        self.bot_connecting
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
    }

    /// Clear the "connecting" flag so future Connect clicks are accepted.
    pub fn clear_connecting(&self) {
        self.bot_connecting.store(false, Ordering::SeqCst);
    }

    /// Whether a connection attempt is currently in progress.
    pub fn is_connecting(&self) -> bool {
        self.bot_connecting.load(Ordering::SeqCst)
    }

    /// Request that the bot disconnect and stop retrying. Set by the
    /// Disconnect button; checked by
    /// [`ConnectionManager::connect`](crate::bot::connection::ConnectionManager::connect)
    /// between reconnection attempts.
    ///
    /// Also cancels the [`CancellationToken`] so any pending reconnect
    /// backoff sleep returns immediately.
    ///
    /// If the bot's ECS handle is present (set on `Event::Spawn`), this
    /// also writes `AppExit::Success` to the ECS World, which causes
    /// `ClientBuilder::start()` to return. Without this, a running
    /// `ClientBuilder::start()` cannot be interrupted by the cancel token
    /// alone, and the bot would stay connected until the server drops it.
    pub fn request_disconnect(&self) {
        self.disconnect_requested.store(true, Ordering::SeqCst);
        self.cancel_token
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .cancel();
        // If the bot's ECS handle is present, write AppExit::Success to
        // trigger ClientBuilder::start() to return (same pattern as
        // Event::Disconnect in bot/events.rs).
        let guard = self.bot_ecs.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(handle) = guard.as_ref() {
            handle.write_app_exit();
        }
    }

    /// Clear the disconnect request (called when starting a new connection).
    pub fn clear_disconnect_request(&self) {
        self.disconnect_requested.store(false, Ordering::SeqCst);
    }

    /// Whether a disconnect has been requested.
    pub fn is_disconnect_requested(&self) -> bool {
        self.disconnect_requested.load(Ordering::SeqCst)
    }

    /// Return a clone of the current [`CancellationToken`].
    ///
    /// The returned token can be awaited (via `cancelled()`) to detect
    /// disconnect requests. Cloning a [`CancellationToken`] is cheap — it
    /// shares the same underlying cancellation state.
    pub fn cancel_token(&self) -> CancellationToken {
        self.cancel_token
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Replace the cancellation token with a fresh one.
    ///
    /// Called at the start of each `connect()` attempt so that a previous
    /// session's cancel (from a prior disconnect) doesn't immediately trip
    /// the new session's backoff sleep.
    pub fn reset_cancel_token(&self) {
        let mut guard = self.cancel_token.lock().unwrap_or_else(|e| e.into_inner());
        *guard = CancellationToken::new();
    }

    /// Update config under a write lock.
    ///
    /// The closure receives `&mut AppConfig`.
    pub fn update_config(&self, f: impl FnOnce(&mut AppConfig)) {
        let mut guard = self.config.write().unwrap_or_else(|e| e.into_inner());
        f(&mut guard);
    }

    /// Read config under a read lock.
    ///
    /// Returns a [`RwLockReadGuard`] — keep the lock short.
    pub fn read_config(&self) -> RwLockReadGuard<'_, AppConfig> {
        self.config.read().unwrap_or_else(|e| e.into_inner())
    }

    /// Read run stats under a read lock.
    ///
    /// Returns a [`RwLockReadGuard`] — keep the lock short.
    /// Atomic counters within [`RunStats`] can still be read without
    /// holding the lock, but [`RunStats::connected_since`] requires it.
    pub fn read_run_stats(&self) -> RwLockReadGuard<'_, RunStats> {
        self.run_stats.read().unwrap_or_else(|e| e.into_inner())
    }

    /// Store (or clear) the container handle.
    ///
    /// If a previous handle was stored, it is dropped and the container
    /// auto-closes.
    pub fn set_container_handle(&self, handle: Option<ContainerHandle>) {
        let mut guard = self
            .container_handle
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        *guard = handle;
    }

    /// Check if a container is currently open without taking the handle.
    ///
    /// Returns `true` if a [`ContainerHandle`] is stored, `false` otherwise.
    /// Unlike [`get_container_handle`](Self::get_container_handle), this does
    /// not consume the handle — the container remains open.
    pub fn has_container_open(&self) -> bool {
        let guard = self
            .container_handle
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        guard.is_some()
    }

    /// Take the container handle out of shared state.
    ///
    /// Returns `None` if no container is currently open.
    /// After this call, [`SharedState`] no longer holds the handle — the
    /// caller owns it and the container will auto-close when the returned
    /// value is dropped.
    pub fn get_container_handle(&self) -> Option<ContainerHandle> {
        let mut guard = self
            .container_handle
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        guard.take()
    }

    /// Borrow the currently-open container handle without removing it.
    ///
    /// The closure receives `&ContainerHandle` and may call its click/shift
    /// methods. Unlike [`get_container_handle`](Self::get_container_handle),
    /// the handle stays in shared state afterwards, so the container remains
    /// open for subsequent operations.
    ///
    /// Returns `None` (and calls the closure with `None`) if no container is
    /// open.
    pub fn with_container_handle<R>(&self, f: impl FnOnce(Option<&ContainerHandle>) -> R) -> R {
        let guard = self
            .container_handle
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        f(guard.as_ref())
    }

    /// Store a chat message, keeping only the last 10.
    pub fn add_chat_message(&self, sender: String, message: String) {
        let mut guard = self.chat_messages.lock().unwrap_or_else(|e| e.into_inner());
        guard.push_back((sender, message));
        while guard.len() > 10 {
            guard.pop_front();
        }
    }

    /// Return a copy of the last 10 chat messages.
    pub fn get_chat_messages(&self) -> Vec<(String, String)> {
        let guard = self.chat_messages.lock().unwrap_or_else(|e| e.into_inner());
        guard.iter().cloned().collect()
    }

    /// Store the last error message reported by the bot/MCP layer.
    ///
    /// Overwrites any previously stored error. The UI reads this to display
    /// a status banner; the MCP layer may include it in tool responses.
    pub fn set_last_error(&self, msg: impl Into<String>) {
        let mut guard = self.last_error.lock().unwrap_or_else(|e| e.into_inner());
        *guard = Some(msg.into());
    }

    /// Clear the last error message (set to `None`).
    ///
    /// Typically called by the UI after the user acknowledges the error,
    /// or by the bot layer when a new connection attempt starts.
    pub fn clear_last_error(&self) {
        let mut guard = self.last_error.lock().unwrap_or_else(|e| e.into_inner());
        *guard = None;
    }

    /// Return a clone of the last error message, if any.
    ///
    /// Returns `None` if no error has been stored or if it was cleared via
    /// [`clear_last_error`](Self::clear_last_error).
    pub fn last_error(&self) -> Option<String> {
        let guard = self.last_error.lock().unwrap_or_else(|e| e.into_inner());
        guard.clone()
    }

    /// Store the bot's ECS handle (set on `Event::Spawn`).
    ///
    /// The handle wraps a closure that writes `AppExit::Success` to the ECS
    /// World, triggering `ClientBuilder::start()` to return. See
    /// [`BotEcsHandle`] for details.
    pub fn set_bot_ecs(&self, handle: BotEcsHandle) {
        let mut guard = self.bot_ecs.lock().unwrap_or_else(|e| e.into_inner());
        *guard = Some(handle);
    }

    /// Clear the bot's ECS handle (set on `Event::Disconnect`).
    ///
    /// After this call, [`request_disconnect`](Self::request_disconnect) will
    /// not attempt to write `AppExit::Success` (the bot is already
    /// disconnecting).
    pub fn clear_bot_ecs(&self) {
        let mut guard = self.bot_ecs.lock().unwrap_or_else(|e| e.into_inner());
        *guard = None;
    }

    /// Return a clone of the bot's ECS handle, if any.
    ///
    /// Returns `None` if no handle is stored (e.g. before `Event::Spawn` or
    /// after `Event::Disconnect`). Cloning is cheap — it clones an [`Arc`].
    pub fn bot_ecs(&self) -> Option<BotEcsHandle> {
        let guard = self.bot_ecs.lock().unwrap_or_else(|e| e.into_inner());
        guard.as_ref().cloned()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::thread;

    // -- Construction --------------------------------------------------------

    #[test]
    fn test_new_state_has_empty_snapshot() {
        let state = SharedState::new(AppConfig::default());
        let snap = state.read_snapshot();
        assert!(snap.blocks.is_empty());
        assert!(snap.entities.is_empty());
        assert_eq!(snap.timestamp, 0);
    }

    #[test]
    fn test_new_state_is_offline() {
        let state = SharedState::new(AppConfig::default());
        assert!(!state.is_online());
    }

    #[test]
    fn test_new_state_has_default_config() {
        let state = SharedState::new(AppConfig::default());
        let cfg = state.read_config();
        assert_eq!(cfg.ai_username, "AI_Bot");
    }

    #[test]
    fn test_new_state_has_default_run_stats() {
        let state = SharedState::new(AppConfig::default());
        let stats = state.run_stats.read().unwrap();
        assert_eq!(stats.commands_processed.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn test_new_state_has_no_container_handle() {
        let state = SharedState::new(AppConfig::default());
        assert!(state.get_container_handle().is_none());
    }

    // -- Snapshot: update / read ---------------------------------------------

    #[test]
    fn test_update_snapshot_is_visible_to_read() {
        let state = SharedState::new(AppConfig::default());

        let new_snap = WorldSnapshot {
            blocks: vec![crate::types::BlockEntry {
                position: crate::types::BlockPos::new(1, 2, 3),
                block_type: "stone".into(),
                block_state: None,
            }],
            entities: vec![],
            self_player: crate::types::SelfPlayer {
                uuid: "u".into(),
                username: "Steve".into(),
                position: crate::types::BlockPos::new(0, 64, 0),
                health: 20.0,
                hunger: 20,
                gamemode: crate::types::GameMode::Survival,
                held_item_slot: 0,
                inventory: Vec::new(),
            },
            timestamp: 42,
            chunk_summary: vec![(0, 0)],
            commands_enabled: None,
        };

        state.update_snapshot(new_snap);
        let snap = state.read_snapshot();
        assert_eq!(snap.timestamp, 42);
        assert_eq!(snap.blocks.len(), 1);
        assert_eq!(snap.blocks[0].block_type, "stone");
    }

    #[test]
    fn test_read_snapshot_returns_arc() {
        let state = SharedState::new(AppConfig::default());
        let snap1 = state.read_snapshot();
        let snap2 = state.read_snapshot();
        // Both Arcs should point to the same allocation (no update yet)
        assert!(Arc::ptr_eq(&snap1, &snap2));
    }

    #[test]
    fn test_concurrent_reads_do_not_block() {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        let mut handles = vec![];

        for _ in 0..10 {
            let s = Arc::clone(&state);
            handles.push(thread::spawn(move || {
                for _ in 0..100 {
                    let _ = s.read_snapshot();
                }
            }));
        }

        for h in handles {
            h.join().unwrap();
        }
    }

    #[test]
    fn test_concurrent_read_while_updating() {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        let mut handles = vec![];

        // Writer thread
        let s_write = Arc::clone(&state);
        handles.push(thread::spawn(move || {
            for i in 0..100 {
                let snap = WorldSnapshot {
                    blocks: vec![],
                    entities: vec![],
                    self_player: crate::types::SelfPlayer {
                        uuid: "u".into(),
                        username: "Steve".into(),
                        position: crate::types::BlockPos::new(0, 64, 0),
                        health: 20.0,
                        hunger: 20,
                        gamemode: crate::types::GameMode::Survival,
                        held_item_slot: 0,
                        inventory: Vec::new(),
                    },
                    timestamp: i,
                    chunk_summary: vec![],
                    commands_enabled: None,
                };
                s_write.update_snapshot(snap);
            }
        }));

        // Reader threads
        for _ in 0..5 {
            let s_read = Arc::clone(&state);
            handles.push(thread::spawn(move || {
                for _ in 0..100 {
                    let _ = s_read.read_snapshot();
                }
            }));
        }

        for h in handles {
            h.join().unwrap();
        }
    }

    // -- Online status -------------------------------------------------------

    #[test]
    fn test_set_online_true() {
        let state = SharedState::new(AppConfig::default());
        state.set_online(true);
        assert!(state.is_online());
    }

    #[test]
    fn test_set_online_false() {
        let state = SharedState::new(AppConfig::default());
        state.set_online(true);
        state.set_online(false);
        assert!(!state.is_online());
    }

    #[test]
    fn test_online_status_atomic_toggle() {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        let mut handles = vec![];

        for _ in 0..10 {
            let s = Arc::clone(&state);
            handles.push(thread::spawn(move || {
                for _ in 0..100 {
                    s.set_online(true);
                    s.set_online(false);
                }
            }));
        }

        for h in handles {
            h.join().unwrap();
        }

        // After all toggles, the value is deterministic because SeqCst
        // ordering makes the last store visible.  We just verify no panic.
        let _ = state.is_online();
    }

    // -- Config RwLock -------------------------------------------------------

    #[test]
    fn test_update_config_changes_value() {
        let state = SharedState::new(AppConfig::default());
        state.update_config(|cfg| {
            cfg.ai_username = "TestBot".into();
        });
        let cfg = state.read_config();
        assert_eq!(cfg.ai_username, "TestBot");
    }

    #[test]
    fn test_concurrent_config_reads() {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        let mut handles = vec![];

        for _ in 0..10 {
            let s = Arc::clone(&state);
            handles.push(thread::spawn(move || {
                let _guard = s.read_config();
                // Hold the guard briefly
            }));
        }

        for h in handles {
            h.join().unwrap();
        }
    }

    #[test]
    fn test_config_read_during_update() {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        let mut handles = vec![];

        // Writer
        let s_write = Arc::clone(&state);
        handles.push(thread::spawn(move || {
            for i in 0..100 {
                s_write.update_config(|cfg| {
                    cfg.mc_port = i as u16;
                });
            }
        }));

        // Readers
        for _ in 0..5 {
            let s_read = Arc::clone(&state);
            handles.push(thread::spawn(move || {
                for _ in 0..100 {
                    let _guard = s_read.read_config();
                }
            }));
        }

        for h in handles {
            h.join().unwrap();
        }
    }

    // -- Container handle ----------------------------------------------------

    #[test]
    fn test_has_container_open_initial() {
        let state = SharedState::new(AppConfig::default());
        assert!(!state.has_container_open());
    }

    #[test]
    fn test_set_container_handle_none_clears_previous() {
        let state = SharedState::new(AppConfig::default());
        // Initially none
        assert!(state.get_container_handle().is_none());
        // Set none explicitly
        state.set_container_handle(None);
        assert!(state.get_container_handle().is_none());
    }

    #[test]
    fn test_container_handle_take_leaves_none() {
        let state = SharedState::new(AppConfig::default());
        state.set_container_handle(None);
        // First take returns None
        assert!(state.get_container_handle().is_none());
        // Second take also returns None
        assert!(state.get_container_handle().is_none());
    }

    // -- Chat messages -------------------------------------------------------

    #[test]
    fn test_add_chat_message() {
        let state = SharedState::new(AppConfig::default());
        state.add_chat_message("Alice".into(), "Hello".into());
        let messages = state.get_chat_messages();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].0, "Alice");
        assert_eq!(messages[0].1, "Hello");
    }

    #[test]
    fn test_chat_message_limit_10() {
        let state = SharedState::new(AppConfig::default());
        for i in 0..15 {
            state.add_chat_message(format!("User{i}"), format!("Msg{i}"));
        }
        let messages = state.get_chat_messages();
        assert_eq!(messages.len(), 10);
        assert_eq!(messages[0].0, "User5");
        assert_eq!(messages[9].0, "User14");
    }

    #[test]
    fn test_chat_messages_empty_by_default() {
        let state = SharedState::new(AppConfig::default());
        assert!(state.get_chat_messages().is_empty());
    }

    // -- last_error -----------------------------------------------------------

    #[test]
    fn test_last_error_initial_none() {
        let state = SharedState::new(AppConfig::default());
        assert!(state.last_error().is_none());
    }

    #[test]
    fn test_set_and_get_last_error() {
        let state = SharedState::new(AppConfig::default());
        state.set_last_error("boom");
        assert_eq!(state.last_error().as_deref(), Some("boom"));
    }

    #[test]
    fn test_clear_last_error() {
        let state = SharedState::new(AppConfig::default());
        state.set_last_error("boom");
        assert!(state.last_error().is_some());
        state.clear_last_error();
        assert!(state.last_error().is_none());
    }

    // -- cancel_token ---------------------------------------------------------

    #[test]
    fn test_cancel_token_initially_not_cancelled() {
        let state = SharedState::new(AppConfig::default());
        let token = state.cancel_token();
        assert!(!token.is_cancelled());
    }

    #[test]
    fn test_request_disconnect_cancels_token() {
        let state = SharedState::new(AppConfig::default());
        let token = state.cancel_token();
        assert!(!token.is_cancelled());
        state.request_disconnect();
        assert!(token.is_cancelled());
    }

    #[test]
    fn test_reset_cancel_token_replaces_with_fresh_one() {
        let state = SharedState::new(AppConfig::default());
        state.request_disconnect();
        let old_token = state.cancel_token();
        assert!(old_token.is_cancelled());

        state.reset_cancel_token();
        let new_token = state.cancel_token();
        assert!(!new_token.is_cancelled());
        // Old token remains cancelled (it's a separate logical token).
        assert!(old_token.is_cancelled());
    }

    #[test]
    fn test_reset_cancel_token_allows_new_session_sleep() {
        // Simulate the connect() flow: reset, take token, request_disconnect
        // cancels the new token (not a stale one).
        let state = SharedState::new(AppConfig::default());
        state.request_disconnect(); // first session cancelled
        state.reset_cancel_token(); // new session
        let token = state.cancel_token();
        assert!(!token.is_cancelled());
        state.request_disconnect();
        assert!(token.is_cancelled());
    }

    // -- bot_ecs --------------------------------------------------------------

    #[test]
    fn test_bot_ecs_initially_none() {
        let state = SharedState::new(AppConfig::default());
        assert!(state.bot_ecs().is_none());
    }

    #[test]
    fn test_set_clear_bot_ecs() {
        let state = SharedState::new(AppConfig::default());
        assert!(state.bot_ecs().is_none());

        let flag = Arc::new(AtomicBool::new(false));
        let flag_clone = Arc::clone(&flag);
        let handle = BotEcsHandle::new(move || {
            flag_clone.store(true, Ordering::SeqCst);
        });
        state.set_bot_ecs(handle);
        assert!(state.bot_ecs().is_some());

        state.clear_bot_ecs();
        assert!(state.bot_ecs().is_none());
    }

    #[test]
    fn test_bot_ecs_clone_invokes_same_closure() {
        // Cloning the handle should share the same closure state.
        let state = SharedState::new(AppConfig::default());
        let flag = Arc::new(AtomicBool::new(false));
        let flag_clone = Arc::clone(&flag);
        state.set_bot_ecs(BotEcsHandle::new(move || {
            flag_clone.store(true, Ordering::SeqCst);
        }));

        let cloned = state.bot_ecs().expect("handle should be present");
        cloned.write_app_exit();
        assert!(flag.load(Ordering::SeqCst));
    }

    #[test]
    fn test_request_disconnect_writes_appexit_when_ecs_present() {
        let state = SharedState::new(AppConfig::default());
        let flag = Arc::new(AtomicBool::new(false));
        let flag_clone = Arc::clone(&flag);
        state.set_bot_ecs(BotEcsHandle::new(move || {
            flag_clone.store(true, Ordering::SeqCst);
        }));
        state.request_disconnect();
        // The closure should have been invoked, setting the flag.
        assert!(flag.load(Ordering::SeqCst));
    }

    #[test]
    fn test_request_disconnect_no_panic_when_ecs_absent() {
        let state = SharedState::new(AppConfig::default());
        // bot_ecs is None — request_disconnect should not panic.
        state.request_disconnect();
        assert!(state.is_disconnect_requested());
    }

    #[test]
    fn test_clear_bot_ecs_no_panic_when_absent() {
        let state = SharedState::new(AppConfig::default());
        // Clearing when already None should not panic.
        state.clear_bot_ecs();
        assert!(state.bot_ecs().is_none());
    }
}
