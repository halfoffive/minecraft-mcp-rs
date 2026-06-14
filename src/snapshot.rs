//! World snapshot with dirty-region optimization.
//!
//! The [`WorldSnapshot`] type lives in `crate::types`; this module adds
//! incremental-update helpers and radius-query methods.

use std::collections::HashSet;
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
}

impl DirtyTracker {
    /// Create a new empty tracker.
    pub fn new() -> Self {
        Self::default()
    }

    /// Mark a single block position as dirty.
    pub fn mark_block_dirty(&mut self, pos: BlockPos) {
        self.dirty_blocks.insert(pos);
    }

    /// Mark an entire chunk as dirty.
    pub fn mark_chunk_dirty(&mut self, pos: (i32, i32)) {
        self.dirty_chunks.insert(pos);
    }

    /// Drain and return the current dirty sets, clearing the tracker.
    pub fn take_dirty_sets(&mut self) -> (HashSet<BlockPos>, HashSet<(i32, i32)>) {
        let blocks = std::mem::take(&mut self.dirty_blocks);
        let chunks = std::mem::take(&mut self.dirty_chunks);
        (blocks, chunks)
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
    new_blocks: Vec<BlockEntry>,
    new_entities: Vec<EntityEntry>,
    new_self_player: Option<SelfPlayer>,
    new_chunk_summary: Option<Vec<(i32, i32)>>,
}

impl SnapshotBuilder {
    /// Start building from an existing snapshot.
    pub fn new(old: WorldSnapshot) -> Self {
        Self {
            old,
            dirty_blocks: HashSet::new(),
            dirty_chunks: HashSet::new(),
            new_blocks: Vec::new(),
            new_entities: Vec::new(),
            new_self_player: None,
            new_chunk_summary: None,
        }
    }

    /// Consume a [`DirtyTracker`] to know which regions changed.
    pub fn with_dirty_tracker(mut self, tracker: &mut DirtyTracker) -> Self {
        let (blocks, chunks) = tracker.take_dirty_sets();
        self.dirty_blocks = blocks;
        self.dirty_chunks = chunks;
        self
    }

    /// Provide the new block data for dirty regions.
    pub fn with_blocks(mut self, blocks: Vec<BlockEntry>) -> Self {
        self.new_blocks = blocks;
        self
    }

    /// Provide a complete replacement entity list.
    pub fn with_entities(mut self, entities: Vec<EntityEntry>) -> Self {
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
        let mut blocks: Vec<BlockEntry> = self
            .old
            .blocks
            .into_iter()
            .filter(|b| {
                let chunk = (b.position.x >> 4, b.position.z >> 4);
                !self.dirty_blocks.contains(&b.position) && !self.dirty_chunks.contains(&chunk)
            })
            .collect();
        blocks.extend(self.new_blocks);

        let entities = if self.new_entities.is_empty() {
            self.old.entities
        } else {
            self.new_entities
        };

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
        }
    }
}

// ═══════════════════════════════════════════════════════════════
// WorldSnapshot radius queries
// ═══════════════════════════════════════════════════════════════

impl WorldSnapshot {
    /// Return all blocks within `radius` blocks (Euclidean distance) of
    /// `center`.
    pub fn blocks_in_radius(&self, center: BlockPos, radius: u32) -> Vec<BlockEntry> {
        let r = radius as f64;
        let r_sq = r * r;
        self.blocks
            .iter()
            .filter(|b| {
                let dx = (b.position.x - center.x) as f64;
                let dy = (b.position.y - center.y) as f64;
                let dz = (b.position.z - center.z) as f64;
                dx * dx + dy * dy + dz * dz <= r_sq
            })
            .cloned()
            .collect()
    }

    /// Return all entities within `radius` blocks (Euclidean distance) of
    /// `center`.
    pub fn entities_in_radius(&self, center: BlockPos, radius: u32) -> Vec<EntityEntry> {
        let r = radius as f64;
        let r_sq = r * r;
        self.entities
            .iter()
            .filter(|e| {
                let dx = (e.position.x - center.x) as f64;
                let dy = (e.position.y - center.y) as f64;
                let dz = (e.position.z - center.z) as f64;
                dx * dx + dy * dy + dz * dz <= r_sq
            })
            .cloned()
            .collect()
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
            },
            timestamp: 0,
            chunk_summary: vec![(0, 0)],
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

    // ── SnapshotBuilder tests ───────────────────────────────

    #[test]
    fn test_builder_no_changes_copies_all() {
        let old = make_snapshot(
            vec![block(BlockPos::new(0, 64, 0), "stone")],
            vec![entity(1, BlockPos::new(0, 64, 0), "zombie")],
        );
        let new = SnapshotBuilder::new(old.clone()).build();
        assert_eq!(new.blocks.len(), old.blocks.len());
        assert_eq!(new.entities.len(), old.entities.len());
        assert_eq!(new.self_player.username, old.self_player.username);
        assert_eq!(new.chunk_summary, old.chunk_summary);
        assert!(new.timestamp >= old.timestamp);
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

        let new = SnapshotBuilder::new(old)
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
                block(BlockPos::new(0, 64, 0), "stone"),   // chunk (0,0)
                block(BlockPos::new(16, 64, 0), "dirt"),  // chunk (1,0)
            ],
            vec![],
        );
        let mut tracker = DirtyTracker::new();
        tracker.mark_chunk_dirty((0, 0));

        let new = SnapshotBuilder::new(old)
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
        let new = SnapshotBuilder::new(old)
            .with_entities(vec![entity(2, BlockPos::new(10, 0, 10), "creeper")])
            .build();
        assert_eq!(new.entities.len(), 1);
        assert_eq!(new.entities[0].id, 2);
        assert_eq!(new.entities[0].entity_type, "creeper");
    }

    #[test]
    fn test_builder_keeps_old_entities_when_none_provided() {
        let old = make_snapshot(vec![], vec![entity(1, BlockPos::new(0, 0, 0), "zombie")]);
        let new = SnapshotBuilder::new(old.clone()).build();
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
        };
        let new = SnapshotBuilder::new(old)
            .with_self_player(new_player.clone())
            .build();
        assert_eq!(new.self_player.username, "Alex");
        assert_eq!(new.self_player.health, 15.0);
        assert_eq!(new.self_player.gamemode, GameMode::Creative);
    }

    #[test]
    fn test_builder_updates_chunk_summary() {
        let old = make_snapshot(vec![], vec![]);
        let new = SnapshotBuilder::new(old)
            .with_chunk_summary(vec![(0, 0), (1, 0), (0, 1)])
            .build();
        assert_eq!(new.chunk_summary.len(), 3);
        assert!(new.chunk_summary.contains(&(1, 0)));
    }

    #[test]
    fn test_builder_dirty_block_and_chunk_together() {
        let old = make_snapshot(
            vec![
                block(BlockPos::new(0, 64, 0), "stone"),   // chunk (0,0)
                block(BlockPos::new(1, 64, 0), "dirt"),    // chunk (0,0)
                block(BlockPos::new(16, 64, 0), "grass"),  // chunk (1,0)
            ],
            vec![],
        );
        let mut tracker = DirtyTracker::new();
        tracker.mark_block_dirty(BlockPos::new(1, 64, 0));
        tracker.mark_chunk_dirty((1, 0));

        let new = SnapshotBuilder::new(old)
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

    // ── Radius query tests ──────────────────────────────────

    #[test]
    fn test_blocks_in_radius_empty() {
        let snapshot = make_snapshot(vec![], vec![]);
        let found = snapshot.blocks_in_radius(BlockPos::new(0, 0, 0), 10);
        assert!(found.is_empty());
    }

    #[test]
    fn test_blocks_in_radius_exact_match() {
        let snapshot = make_snapshot(
            vec![
                block(BlockPos::new(0, 0, 0), "origin"),
                block(BlockPos::new(3, 0, 0), "three_x"),
                block(BlockPos::new(0, 4, 0), "four_y"),
            ],
            vec![],
        );
        let found = snapshot.blocks_in_radius(BlockPos::new(0, 0, 0), 5);
        assert_eq!(found.len(), 3);
    }

    #[test]
    fn test_blocks_in_radius_excludes_outside() {
        let snapshot = make_snapshot(
            vec![
                block(BlockPos::new(0, 0, 0), "origin"),
                block(BlockPos::new(10, 0, 0), "far"),
            ],
            vec![],
        );
        let found = snapshot.blocks_in_radius(BlockPos::new(0, 0, 0), 5);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].block_type, "origin");
    }

    #[test]
    fn test_blocks_in_radius_3d_diagonal() {
        // Distance from (0,0,0) to (3,3,3) = sqrt(27) ≈ 5.196
        let snapshot = make_snapshot(
            vec![block(BlockPos::new(3, 3, 3), "diagonal")],
            vec![],
        );
        let found = snapshot.blocks_in_radius(BlockPos::new(0, 0, 0), 5);
        assert!(found.is_empty()); // 5.196 > 5
        let found = snapshot.blocks_in_radius(BlockPos::new(0, 0, 0), 6);
        assert_eq!(found.len(), 1);
    }

    #[test]
    fn test_entities_in_radius() {
        let snapshot = make_snapshot(
            vec![],
            vec![
                entity(1, BlockPos::new(0, 0, 0), "zombie"),
                entity(2, BlockPos::new(100, 0, 0), "skeleton"),
            ],
        );
        let found = snapshot.entities_in_radius(BlockPos::new(0, 0, 0), 50);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, 1);
    }

    #[test]
    fn test_entities_in_radius_zero_radius() {
        let snapshot = make_snapshot(
            vec![],
            vec![entity(1, BlockPos::new(0, 0, 0), "zombie")],
        );
        let found = snapshot.entities_in_radius(BlockPos::new(0, 0, 0), 0);
        // Distance is exactly 0, so it should match
        assert_eq!(found.len(), 1);
    }

    #[test]
    fn test_blocks_in_radius_negative_coords() {
        let snapshot = make_snapshot(
            vec![block(BlockPos::new(-5, 0, -5), "neg")],
            vec![],
        );
        let found = snapshot.blocks_in_radius(BlockPos::new(0, 0, 0), 8);
        // Distance = sqrt(50) ≈ 7.07 < 8
        assert_eq!(found.len(), 1);
    }
}
