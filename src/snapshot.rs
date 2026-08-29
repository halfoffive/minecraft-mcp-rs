//! World snapshot with dirty-region optimization.
//!
//! The [`WorldSnapshot`] type lives in `crate::types`; this module adds
//! incremental-update helpers (`DirtyTracker`, `SnapshotBuilder`).

use std::collections::{HashMap, HashSet};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::types::{BlockEntry, BlockPos, EntityEntry, SelfPlayer, WorldSnapshot};

// ═══════════════════════════════════════════════════════════════
// DirtyTracker
// ═══════════════════════════════════════════════════════════════

/// Tracks which blocks and chunks have changed since the last snapshot.
#[derive(Debug, Clone, Default)]
pub struct DirtyTracker {
    dirty_blocks: HashSet<BlockPos>,
    dirty_chunks: HashSet<(i32, i32)>,
    /// Spatial index from dirty chunk to the set of dirty block positions
    /// within that chunk. A chunk may be present with an empty set when the
    /// entire chunk is marked dirty via `mark_chunk_dirty`.
    chunk_index: HashMap<(i32, i32), HashSet<BlockPos>>,
}

impl DirtyTracker {
    /// Create a new empty tracker.
    pub fn new() -> Self {
        Self::default()
    }

    /// Mark a single block position as dirty.
    pub fn mark_block_dirty(&mut self, pos: BlockPos) {
        self.dirty_blocks.insert(pos);
        let chunk = (pos.x >> 4, pos.z >> 4);
        self.chunk_index.entry(chunk).or_default().insert(pos);
    }

    /// Mark an entire chunk as dirty.
    pub fn mark_chunk_dirty(&mut self, pos: (i32, i32)) {
        self.dirty_chunks.insert(pos);
        self.chunk_index.entry(pos).or_default();
    }

    /// Drain and return the current dirty sets, clearing the tracker.
    pub fn take_dirty_sets(&mut self) -> (HashSet<BlockPos>, HashSet<(i32, i32)>) {
        let blocks = std::mem::take(&mut self.dirty_blocks);
        let chunks = std::mem::take(&mut self.dirty_chunks);
        self.chunk_index.clear();
        (blocks, chunks)
    }

    /// Returns a reference to the chunk -> dirty block positions index.
    pub fn dirty_chunk_index(&self) -> &HashMap<(i32, i32), HashSet<BlockPos>> {
        &self.chunk_index
    }

    /// Returns true if no dirty regions are tracked.
    pub fn is_empty(&self) -> bool {
        self.dirty_blocks.is_empty() && self.dirty_chunks.is_empty()
    }
}

// ═══════════════════════════════════════════════════════════════
// SnapshotBuilder
// ═══════════════════════════════════════════════════════════════

/// Builder for producing an updated [`WorldSnapshot`] from an old one plus
/// dirty-region changes.
///
/// Non-dirty blocks are copied from the old snapshot; dirty blocks are
/// replaced by the new block list.  Entities are **not** incrementally
/// tracked in v1 — they are replaced in full when provided.
#[derive(Debug, Clone)]
pub struct SnapshotBuilder {
    old: WorldSnapshot,
    dirty_blocks: HashSet<BlockPos>,
    dirty_chunks: HashSet<(i32, i32)>,
    dirty_chunk_index: HashMap<(i32, i32), HashSet<BlockPos>>,
    new_blocks: Vec<BlockEntry>,
    new_entities: Option<Vec<EntityEntry>>,
    new_self_player: Option<SelfPlayer>,
    new_chunk_summary: Option<Vec<(i32, i32)>>,
}

impl SnapshotBuilder {
    /// Start building from the previous snapshot's block list (production
    /// path).
    ///
    /// Only the blocks are carried over: the per-tick snapshot path always
    /// replaces every other field via the `with_*` setters, so the previous
    /// full snapshot — including its `block_index` HashMap and every field
    /// the builder would otherwise deep-clone — is never copied each tick.
    /// Fields not set by `with_*` default to their `WorldSnapshot` defaults.
    pub fn new(old_blocks: Vec<BlockEntry>) -> Self {
        Self::from_old(WorldSnapshot {
            blocks: old_blocks,
            ..WorldSnapshot::default()
        })
    }

    /// Start building from a full previous snapshot (test / fallback path).
    ///
    /// All fields not replaced by the `with_*` setters fall back to the old
    /// snapshot's values.
    pub fn from_old(old: WorldSnapshot) -> Self {
        Self {
            old,
            dirty_blocks: HashSet::new(),
            dirty_chunks: HashSet::new(),
            dirty_chunk_index: HashMap::new(),
            new_blocks: Vec::new(),
            new_entities: None,
            new_self_player: None,
            new_chunk_summary: None,
        }
    }

    /// Consume a [`DirtyTracker`] to know which regions changed.
    pub fn with_dirty_tracker(mut self, tracker: &mut DirtyTracker) -> Self {
        let chunk_index = tracker.dirty_chunk_index().clone();
        let (blocks, chunks) = tracker.take_dirty_sets();
        self.dirty_blocks = blocks;
        self.dirty_chunks = chunks;
        self.dirty_chunk_index = chunk_index;
        self
    }

    /// Provide the new block data for dirty regions.
    pub fn with_blocks(mut self, blocks: Vec<BlockEntry>) -> Self {
        self.new_blocks = blocks;
        self
    }

    /// Provide a complete replacement entity list.
    ///
    /// Pass `Some(vec)` to replace the entity list — an empty vec clears it.
    /// Pass `None` (or skip calling this method) to keep the old entities
    /// unchanged.
    pub fn with_entities(mut self, entities: Option<Vec<EntityEntry>>) -> Self {
        self.new_entities = entities;
        self
    }

    /// Provide updated self-player info.
    pub fn with_self_player(mut self, player: SelfPlayer) -> Self {
        self.new_self_player = Some(player);
        self
    }

    /// Provide an updated chunk summary.
    pub fn with_chunk_summary(mut self, chunks: Vec<(i32, i32)>) -> Self {
        self.new_chunk_summary = Some(chunks);
        self
    }

    /// Produce the final [`WorldSnapshot`].
    ///
    /// Blocks from the old snapshot that fall inside a dirty block position
    /// or a dirty chunk are removed; the new block list is appended.
    /// All other fields use the new data when provided, otherwise fall back
    /// to the old snapshot.
    pub fn build(self) -> WorldSnapshot {
        let mut blocks: Vec<BlockEntry> =
            Vec::with_capacity(self.old.blocks.len() + self.new_blocks.len());

        for b in self.old.blocks {
            let chunk = (b.position.x >> 4, b.position.z >> 4);
            // The dirty_chunk_index probe here was redundant (2026-08-30
            // review): both `mark_block_dirty` and `mark_chunk_dirty`
            // insert the block's/whole chunk into the index, so every
            // chunk touched by `dirty_blocks` or `dirty_chunks` is
            // guaranteed a key — the membership tests below imply it.
            // Pinned by test_dirty_chunk_index_covers_dirty_regions.
            if self.dirty_blocks.contains(&b.position) || self.dirty_chunks.contains(&chunk) {
                continue;
            }
            blocks.push(b);
        }
        blocks.extend(self.new_blocks);

        let block_index: HashMap<BlockPos, usize> = blocks
            .iter()
            .enumerate()
            .map(|(i, b)| (b.position, i))
            .collect();

        let entities = self.new_entities.unwrap_or(self.old.entities);

        let self_player = self.new_self_player.unwrap_or(self.old.self_player);
        let chunk_summary = self.new_chunk_summary.unwrap_or(self.old.chunk_summary);
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        WorldSnapshot {
            blocks,
            entities,
            self_player,
            timestamp,
            chunk_summary,
            commands_enabled: self.old.commands_enabled,
            block_index,
            // Production callers immediately store this through
            // `SharedState::update_snapshot`, which overwrites the sequence
            // with a monotonic revision. Direct `SnapshotBuilder::build`
            // callers (tests) do not need a non-zero value.
            snapshot_seq: 0,
        }
    }
}

// ═══════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{BlockEntry, BlockPos, EntityEntry, GameMode, SelfPlayer};

    // ── Helpers ─────────────────────────────────────────────

    fn make_snapshot(blocks: Vec<BlockEntry>, entities: Vec<EntityEntry>) -> WorldSnapshot {
        WorldSnapshot {
            blocks,
            entities,
            self_player: SelfPlayer {
                uuid: "uuid".into(),
                username: "Steve".into(),
                position: BlockPos::new(0, 64, 0),
                health: 20.0,
                hunger: 20,
                gamemode: GameMode::Survival,
                held_item_slot: 0,
                inventory: Vec::new(),
                position_precise: None,
                yaw: None,
            },
            timestamp: 0,
            chunk_summary: vec![(0, 0)],
            commands_enabled: None,
            ..Default::default()
        }
    }

    fn block(pos: BlockPos, name: &str) -> BlockEntry {
        BlockEntry {
            position: pos,
            block_type: name.into(),
            block_state: None,
        }
    }

    fn entity(id: u32, pos: BlockPos, name: &str) -> EntityEntry {
        EntityEntry {
            id,
            uuid: format!("uuid-{id}"),
            entity_type: name.into(),
            position: pos,
            display_name: None,
            health: Some(20.0),
        }
    }

    // ── DirtyTracker tests ──────────────────────────────────

    #[test]
    fn test_dirty_tracker_new_is_empty() {
        let tracker = DirtyTracker::new();
        assert!(tracker.is_empty());
    }

    #[test]
    fn test_dirty_tracker_mark_block() {
        let mut tracker = DirtyTracker::new();
        tracker.mark_block_dirty(BlockPos::new(1, 2, 3));
        assert!(!tracker.is_empty());
        let (blocks, chunks) = tracker.take_dirty_sets();
        assert!(blocks.contains(&BlockPos::new(1, 2, 3)));
        assert!(chunks.is_empty());
    }

    #[test]
    fn test_dirty_tracker_mark_chunk() {
        let mut tracker = DirtyTracker::new();
        tracker.mark_chunk_dirty((5, -3));
        assert!(!tracker.is_empty());
        let (blocks, chunks) = tracker.take_dirty_sets();
        assert!(blocks.is_empty());
        assert!(chunks.contains(&(5, -3)));
    }

    #[test]
    fn test_dirty_tracker_take_clears() {
        let mut tracker = DirtyTracker::new();
        tracker.mark_block_dirty(BlockPos::new(0, 0, 0));
        tracker.mark_chunk_dirty((1, 1));
        let (b1, c1) = tracker.take_dirty_sets();
        assert_eq!(b1.len(), 1);
        assert_eq!(c1.len(), 1);
        let (b2, c2) = tracker.take_dirty_sets();
        assert!(b2.is_empty());
        assert!(c2.is_empty());
        assert!(tracker.is_empty());
    }

    #[test]
    fn test_dirty_tracker_multiple_blocks() {
        let mut tracker = DirtyTracker::new();
        tracker.mark_block_dirty(BlockPos::new(0, 0, 0));
        tracker.mark_block_dirty(BlockPos::new(1, 0, 0));
        tracker.mark_block_dirty(BlockPos::new(0, 0, 0)); // duplicate
        let (blocks, _) = tracker.take_dirty_sets();
        assert_eq!(blocks.len(), 2);
    }

    #[test]
    fn test_dirty_tracker_chunk_index() {
        let mut tracker = DirtyTracker::new();
        tracker.mark_block_dirty(BlockPos::new(0, 64, 0)); // chunk (0,0)
        tracker.mark_block_dirty(BlockPos::new(16, 64, 0)); // chunk (1,0)
        tracker.mark_chunk_dirty((2, 2)); // empty-set entry

        let index = tracker.dirty_chunk_index();
        assert_eq!(index.len(), 3);
        assert!(
            index
                .get(&(0, 0))
                .unwrap()
                .contains(&BlockPos::new(0, 64, 0))
        );
        assert!(
            index
                .get(&(1, 0))
                .unwrap()
                .contains(&BlockPos::new(16, 64, 0))
        );
        assert!(index.get(&(2, 2)).unwrap().is_empty());

        let (_, _) = tracker.take_dirty_sets();
        assert!(tracker.dirty_chunk_index().is_empty());
    }

    // ── SnapshotBuilder tests ───────────────────────────────

    #[test]
    #[test]
    fn test_dirty_chunk_index_covers_dirty_regions() {
        // 2026-08-30 review invariant: every chunk touched by a dirty
        // block, and every fully-dirty chunk, has a key in the index —
        // which is what lets SnapshotBuilder::build skip the redundant
        // `dirty_chunk_index.contains_key` probe.
        let mut tracker = DirtyTracker::new();
        tracker.mark_block_dirty(BlockPos::new(5, 64, 7)); // chunk (0, 0)
        tracker.mark_block_dirty(BlockPos::new(-3, 64, -20)); // chunk (-1, -2)
        tracker.mark_chunk_dirty((9, 9));
        // The index must be probed BEFORE take_dirty_sets: the take clears
        // it (that is exactly how SnapshotBuilder receives all three sets
        // consistently — cloned together at construction).
        let (blocks, chunks, index_keys) = {
            let keys: std::collections::HashSet<_> =
                tracker.dirty_chunk_index().keys().copied().collect();
            let (blocks, chunks) = tracker.take_dirty_sets();
            (blocks, chunks, keys)
        };
        for pos in &blocks {
            let chunk = (pos.x >> 4, pos.z >> 4);
            assert!(
                index_keys.contains(&chunk),
                "chunk {chunk:?} of dirty block {pos:?} must be indexed"
            );
        }
        for chunk in &chunks {
            assert!(
                index_keys.contains(chunk),
                "fully-dirty chunk {chunk:?} must be indexed"
            );
        }
    }

    fn test_builder_no_changes_copies_all() {
        let old = make_snapshot(
            vec![block(BlockPos::new(0, 64, 0), "stone")],
            vec![entity(1, BlockPos::new(0, 64, 0), "zombie")],
        );
        let new = SnapshotBuilder::from_old(old.clone()).build();
        assert_eq!(new.blocks.len(), old.blocks.len());
        assert_eq!(new.entities.len(), old.entities.len());
        assert_eq!(new.self_player.username, old.self_player.username);
        assert_eq!(new.chunk_summary, old.chunk_summary);
        assert!(new.timestamp >= old.timestamp);
    }

    /// The production `new` constructor carries over the block list and
    /// replaces dirty regions, exactly like `from_old` — but unset fields
    /// fall back to `WorldSnapshot::default()` instead of the old snapshot.
    #[test]
    fn test_builder_new_production_constructor() {
        let old = make_snapshot(
            vec![
                block(BlockPos::new(0, 64, 0), "stone"),
                block(BlockPos::new(1, 64, 0), "dirt"),
            ],
            vec![entity(1, BlockPos::new(0, 64, 0), "zombie")],
        );
        let mut tracker = DirtyTracker::new();
        tracker.mark_block_dirty(BlockPos::new(0, 64, 0));

        let new = SnapshotBuilder::new(old.blocks.clone())
            .with_dirty_tracker(&mut tracker)
            .with_blocks(vec![block(BlockPos::new(0, 64, 0), "gold_block")])
            .build();

        // Dirty filtering works on the carried-over blocks.
        assert_eq!(new.blocks.len(), 2);
        let types: Vec<_> = new.blocks.iter().map(|b| b.block_type.clone()).collect();
        assert!(types.contains(&"gold_block".into()));
        assert!(types.contains(&"dirt".into()));
        assert!(!types.contains(&"stone".into()));

        // Unset fields default instead of falling back to the old snapshot.
        assert!(new.entities.is_empty());
        assert_eq!(new.self_player.username, "");
        assert!(new.chunk_summary.is_empty());
        assert_eq!(new.commands_enabled, None);
    }

    #[test]
    fn test_builder_replaces_dirty_block() {
        let old = make_snapshot(
            vec![
                block(BlockPos::new(0, 64, 0), "stone"),
                block(BlockPos::new(1, 64, 0), "dirt"),
            ],
            vec![],
        );
        let mut tracker = DirtyTracker::new();
        tracker.mark_block_dirty(BlockPos::new(0, 64, 0));

        let new = SnapshotBuilder::from_old(old)
            .with_dirty_tracker(&mut tracker)
            .with_blocks(vec![block(BlockPos::new(0, 64, 0), "gold_block")])
            .build();

        assert_eq!(new.blocks.len(), 2);
        let types: Vec<_> = new.blocks.iter().map(|b| b.block_type.clone()).collect();
        assert!(types.contains(&"gold_block".into()));
        assert!(types.contains(&"dirt".into()));
        assert!(!types.contains(&"stone".into()));
    }

    #[test]
    fn test_builder_replaces_dirty_chunk() {
        // Chunk (0,0) contains positions where x>>4==0 and z>>4==0
        let old = make_snapshot(
            vec![
                block(BlockPos::new(0, 64, 0), "stone"), // chunk (0,0)
                block(BlockPos::new(16, 64, 0), "dirt"), // chunk (1,0)
            ],
            vec![],
        );
        let mut tracker = DirtyTracker::new();
        tracker.mark_chunk_dirty((0, 0));

        let new = SnapshotBuilder::from_old(old)
            .with_dirty_tracker(&mut tracker)
            .with_blocks(vec![block(BlockPos::new(0, 64, 0), "gold_block")])
            .build();

        assert_eq!(new.blocks.len(), 2);
        let types: Vec<_> = new.blocks.iter().map(|b| b.block_type.clone()).collect();
        assert!(types.contains(&"gold_block".into()));
        assert!(types.contains(&"dirt".into()));
        assert!(!types.contains(&"stone".into()));
    }

    #[test]
    fn test_builder_replaces_entities() {
        let old = make_snapshot(vec![], vec![entity(1, BlockPos::new(0, 0, 0), "zombie")]);
        let new = SnapshotBuilder::from_old(old)
            .with_entities(Some(vec![entity(2, BlockPos::new(10, 0, 10), "creeper")]))
            .build();
        assert_eq!(new.entities.len(), 1);
        assert_eq!(new.entities[0].id, 2);
        assert_eq!(new.entities[0].entity_type, "creeper");
    }

    #[test]
    fn test_builder_clears_entities_with_empty_vec() {
        let old = make_snapshot(vec![], vec![entity(1, BlockPos::new(0, 0, 0), "zombie")]);
        let new = SnapshotBuilder::from_old(old)
            .with_entities(Some(Vec::new()))
            .build();
        assert!(new.entities.is_empty());
    }

    #[test]
    fn test_builder_with_entities_none_keeps_old() {
        let old = make_snapshot(vec![], vec![entity(1, BlockPos::new(0, 0, 0), "zombie")]);
        let new = SnapshotBuilder::from_old(old.clone())
            .with_entities(None)
            .build();
        assert_eq!(new.entities.len(), 1);
        assert_eq!(new.entities[0].id, 1);
    }

    #[test]
    fn test_builder_keeps_old_entities_when_none_provided() {
        let old = make_snapshot(vec![], vec![entity(1, BlockPos::new(0, 0, 0), "zombie")]);
        let new = SnapshotBuilder::from_old(old.clone()).build();
        assert_eq!(new.entities.len(), 1);
        assert_eq!(new.entities[0].id, 1);
    }

    #[test]
    fn test_builder_updates_self_player() {
        let old = make_snapshot(vec![], vec![]);
        let new_player = SelfPlayer {
            uuid: "new-uuid".into(),
            username: "Alex".into(),
            position: BlockPos::new(100, 64, 200),
            health: 15.0,
            hunger: 18,
            gamemode: GameMode::Creative,
            held_item_slot: 3,
            inventory: Vec::new(),
            position_precise: None,
            yaw: None,
        };
        let new = SnapshotBuilder::from_old(old)
            .with_self_player(new_player.clone())
            .build();
        assert_eq!(new.self_player.username, "Alex");
        assert_eq!(new.self_player.health, 15.0);
        assert_eq!(new.self_player.gamemode, GameMode::Creative);
    }

    #[test]
    fn test_builder_updates_chunk_summary() {
        let old = make_snapshot(vec![], vec![]);
        let new = SnapshotBuilder::from_old(old)
            .with_chunk_summary(vec![(0, 0), (1, 0), (0, 1)])
            .build();
        assert_eq!(new.chunk_summary.len(), 3);
        assert!(new.chunk_summary.contains(&(1, 0)));
    }

    #[test]
    fn test_builder_dirty_block_and_chunk_together() {
        let old = make_snapshot(
            vec![
                block(BlockPos::new(0, 64, 0), "stone"),  // chunk (0,0)
                block(BlockPos::new(1, 64, 0), "dirt"),   // chunk (0,0)
                block(BlockPos::new(16, 64, 0), "grass"), // chunk (1,0)
            ],
            vec![],
        );
        let mut tracker = DirtyTracker::new();
        tracker.mark_block_dirty(BlockPos::new(1, 64, 0));
        tracker.mark_chunk_dirty((1, 0));

        let new = SnapshotBuilder::from_old(old)
            .with_dirty_tracker(&mut tracker)
            .with_blocks(vec![
                block(BlockPos::new(1, 64, 0), "diamond_block"),
                block(BlockPos::new(16, 64, 0), "emerald_block"),
            ])
            .build();

        assert_eq!(new.blocks.len(), 3);
        let types: Vec<_> = new.blocks.iter().map(|b| b.block_type.clone()).collect();
        assert!(types.contains(&"stone".into()));
        assert!(types.contains(&"diamond_block".into()));
        assert!(types.contains(&"emerald_block".into()));
        assert!(!types.contains(&"dirt".into()));
        assert!(!types.contains(&"grass".into()));
    }

    #[test]
    fn test_builder_skips_clean_chunks() {
        // Place many blocks in a clean chunk far from the dirty block. If the
        // implementation accidentally scanned all old blocks, these would still
        // be preserved, but this test documents that they are untouched.
        let mut old_blocks = vec![
            block(BlockPos::new(0, 64, 0), "target"), // chunk (0,0)
        ];
        for i in 1..=100 {
            // chunk (10,10) is clean
            old_blocks.push(block(BlockPos::new(160 + i, 64, 160 + i), "clean"));
        }

        let old = make_snapshot(old_blocks, vec![]);
        let mut tracker = DirtyTracker::new();
        tracker.mark_block_dirty(BlockPos::new(0, 64, 0));

        let new = SnapshotBuilder::from_old(old)
            .with_dirty_tracker(&mut tracker)
            .with_blocks(vec![block(BlockPos::new(0, 64, 0), "replaced")])
            .build();

        assert_eq!(new.blocks.len(), 101);
        let types: Vec<_> = new.blocks.iter().map(|b| b.block_type.clone()).collect();
        assert!(types.contains(&"replaced".into()));
        assert!(!types.contains(&"target".into()));
        assert_eq!(
            new.blocks
                .iter()
                .filter(|b| b.block_type == "clean")
                .count(),
            100
        );
    }

    #[test]
    fn test_builder_dirty_chunk_replaces_all_in_chunk() {
        let old = make_snapshot(
            vec![
                block(BlockPos::new(0, 64, 0), "a"),     // chunk (0,0)
                block(BlockPos::new(15, 64, 15), "b"),   // chunk (0,0)
                block(BlockPos::new(16, 64, 0), "c"),    // chunk (1,0)
                block(BlockPos::new(160, 64, 160), "d"), // chunk (10,10)
            ],
            vec![],
        );
        let mut tracker = DirtyTracker::new();
        tracker.mark_chunk_dirty((0, 0));

        let new = SnapshotBuilder::from_old(old)
            .with_dirty_tracker(&mut tracker)
            .with_blocks(vec![block(BlockPos::new(0, 64, 0), "new_a")])
            .build();

        assert_eq!(new.blocks.len(), 3);
        let types: Vec<_> = new.blocks.iter().map(|b| b.block_type.clone()).collect();
        assert!(types.contains(&"new_a".into()));
        assert!(types.contains(&"c".into()));
        assert!(types.contains(&"d".into()));
        assert!(!types.contains(&"a".into()));
        assert!(!types.contains(&"b".into()));
    }

    #[test]
    fn test_builder_preserves_commands_enabled() {
        let mut old = make_snapshot(vec![], vec![]);
        old.commands_enabled = Some(true);
        let new = SnapshotBuilder::from_old(old.clone()).build();
        assert_eq!(new.commands_enabled, Some(true));
    }
}
