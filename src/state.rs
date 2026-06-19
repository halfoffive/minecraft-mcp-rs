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

use crate::config::{AppConfig, RunStats};
use crate::types::WorldSnapshot;

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
    /// Handle to the currently open container (if any).
    ///
    /// Stored behind `Mutex<Option<_>>` because [`ContainerHandle`] auto-closes
    /// on [`Drop`], so we must ensure only one owner exists at a time.
    container_handle: Arc<Mutex<Option<ContainerHandle>>>,
    /// Last 10 chat messages received from the server.
    ///
    /// Each entry is `(sender, message)`. Stored behind a `Mutex` because the
    /// bot event handler writes to it from azalea's ECS thread.
    chat_messages: Mutex<VecDeque<(String, String)>>,
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
            },
            timestamp: 0,
            chunk_summary: vec![],
        };

        Self {
            world_snapshot: ArcSwap::from_pointee(empty_snapshot),
            config: RwLock::new(config),
            run_stats: RwLock::new(RunStats::default()),
            bot_online: AtomicBool::new(false),
            container_handle: Arc::new(Mutex::new(None)),
            chat_messages: Mutex::new(VecDeque::new()),
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
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;
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
            },
            timestamp: 42,
            chunk_summary: vec![(0, 0)],
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
                    },
                    timestamp: i,
                    chunk_summary: vec![],
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
}
