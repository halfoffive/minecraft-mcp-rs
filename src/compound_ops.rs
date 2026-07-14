//! Compound operation state machines for multi-step bot actions.
//!
//! Each state machine is a pure function — no I/O, no side effects.
//! The bot executor (Task 19) will use these to drive compound operations
//! by calling `current_action()` to get the next `BotCommand` and
//! `advance()` to transition states based on game events.

use crate::error::BotError;
use crate::types::{BlockPos, BotCommand, ToolType};

// ═══════════════════════════════════════════════════════════════
// OperationState
// ═══════════════════════════════════════════════════════════════

/// The lifecycle state of a compound operation.
#[derive(Debug, Clone, PartialEq)]
pub enum OperationState {
    /// Operation has not started.
    Idle,
    /// Bot is moving toward the target position.
    MovingToTarget,
    /// Bot is selecting the right tool/item in its hotbar.
    EquippingTool,
    /// Bot is performing the main action (mining, placing, opening).
    ExecutingAction,
    /// Bot is waiting for the action to complete (e.g. block break animation).
    WaitingForResult,
    /// Operation finished successfully.
    Completed,
    /// Operation failed with an error.
    Failed(BotError),
}

// ═══════════════════════════════════════════════════════════════
// OperationEvent
// ═══════════════════════════════════════════════════════════════

/// Events that drive state transitions.
#[derive(Debug, Clone, PartialEq)]
pub enum OperationEvent {
    /// Begin the operation.
    Start,
    /// Bot has arrived at the target position.
    Arrived,
    /// The required tool/item is now equipped.
    ToolEquipped,
    /// The required tool is already present in the bot's inventory
    /// (hotbar or main inventory) but cannot be auto-switched to from
    /// `EquippingTool` (e.g. it lives in the main inventory, slot 9-35,
    /// and `SwitchHotbarSlot` only accepts 0-8). The state machine should
    /// skip the equipping step and proceed directly to executing the main
    /// action, mining with whatever the bot is already holding.
    ///
    /// Added by Task 1.5 (P0-#3 + P1-#6) so the executor can detect this
    /// condition and skip a guaranteed-failed `EquipTool` round-trip.
    ToolAlreadyInInventory,
    /// The main action has been initiated.
    ActionStarted,
    /// The block has been broken.
    BlockBroken,
    /// The block has been placed.
    BlockPlaced,
    /// The container has been opened.
    ContainerOpened,
    /// An error occurred — operation should fail.
    Failed(BotError),
}

// ═══════════════════════════════════════════════════════════════
// Standable-neighbour lookup
// ═══════════════════════════════════════════════════════════════

/// Horizontal neighbour offsets scanned by [`find_standable_neighbor`].
///
/// Eight directions: 4 orthogonal (±1 along a single axis) plus 4 diagonal
/// (±1 along both X and Z). Diagonals matter when a block is wedged in a
/// corner where only one diagonal neighbour is reachable (e.g. inside a
/// 1-block-wide gap between two walls).
const STANDABLE_OFFSETS_XZ: [(i32, i32); 8] = [
    (-1, -1),
    (-1, 0),
    (-1, 1),
    (0, -1),
    (0, 1),
    (1, -1),
    (1, 0),
    (1, 1),
];

/// Find a standable position adjacent to `target` (±1 X/Z, same Y or ±1 Y).
///
/// A position is standable if the block at that position is air (or absent
/// from the snapshot, which is treated as air) and the block below it is
/// solid (non-air). Returns `None` if no such position exists in the
/// snapshot.
///
/// Scans 8 horizontal neighbours (4 orthogonal + 4 diagonal) × 3 Y levels
/// (priority: same Y first, then y+1, then y-1). Diagonals are necessary
/// when the target is wedged in a 1-block gap where only a diagonal cell
/// is reachable. Looks up blocks via `WorldSnapshot::block_index` — O(1)
/// per check.
pub fn find_standable_neighbor(
    snapshot: &crate::types::WorldSnapshot,
    target: crate::types::BlockPos,
) -> Option<crate::types::BlockPos> {
    // Check 8 horizontal neighbours at 3 Y levels (y-1, y, y+1).
    // Priority: same Y first, then y+1 (step up), then y-1 (step down).
    let offsets_y: [i32; 3] = [0, 1, -1];

    for &dy in &offsets_y {
        for &(dx, dz) in &STANDABLE_OFFSETS_XZ {
            let pos = crate::types::BlockPos::new(target.x + dx, target.y + dy, target.z + dz);
            let below = crate::types::BlockPos::new(pos.x, pos.y - 1, pos.z);

            // A position is standable if it is air (or absent from the
            // snapshot, which we treat as air) and the block below it is
            // solid (non-air). Use the O(1) `block_index` instead of a
            // linear `blocks.iter().find()` scan.
            let pos_is_clear = snapshot
                .block_index
                .get(&pos)
                .map(|&idx| snapshot.blocks[idx].block_type.eq_ignore_ascii_case("air"))
                .unwrap_or(true);
            let below_is_solid = snapshot
                .block_index
                .get(&below)
                .map(|&idx| !snapshot.blocks[idx].block_type.eq_ignore_ascii_case("air"))
                .unwrap_or(false);

            if pos_is_clear && below_is_solid {
                return Some(pos);
            }
        }
    }
    None
}

// ═══════════════════════════════════════════════════════════════
// MineBlockOperation
// ═══════════════════════════════════════════════════════════════

/// State machine for mining a single block.
///
/// Lifecycle:
/// Idle → MovingToTarget → EquippingTool → ExecutingAction → WaitingForResult → Completed
pub struct MineBlockOperation {
    pub target: BlockPos,
    pub tool: ToolType,
}

impl MineBlockOperation {
    pub fn new(target: BlockPos, tool: ToolType) -> Self {
        Self { target, tool }
    }

    /// Advance the state machine given the current state and an event.
    pub fn advance(&self, state: OperationState, event: OperationEvent) -> OperationState {
        match (&state, &event) {
            // Happy path
            (OperationState::Idle, OperationEvent::Start) => OperationState::MovingToTarget,
            (OperationState::MovingToTarget, OperationEvent::Arrived) => {
                OperationState::EquippingTool
            }
            // Task 1.5 (P0-#3 + P1-#6): when the best tool is in the bot's
            // inventory but cannot be auto-equipped (e.g. main-inventory
            // slot, where `SwitchHotbarSlot` only accepts 0-8), skip the
            // `EquippingTool` step and go straight to executing the action.
            // The bot will mine with whatever it is already holding.
            (OperationState::MovingToTarget, OperationEvent::ToolAlreadyInInventory) => {
                OperationState::ExecutingAction
            }
            (OperationState::EquippingTool, OperationEvent::ToolEquipped) => {
                OperationState::ExecutingAction
            }
            (OperationState::ExecutingAction, OperationEvent::ActionStarted) => {
                OperationState::WaitingForResult
            }
            (OperationState::WaitingForResult, OperationEvent::BlockBroken) => {
                OperationState::Completed
            }

            // Failure from any state
            (_, OperationEvent::Failed(err)) => OperationState::Failed(err.clone()),

            // Terminal states are sticky
            (OperationState::Completed | OperationState::Failed(_), _) => state,

            // Invalid transition — stay in current state
            _ => state,
        }
    }

    /// Return the `BotCommand` that should be issued for the current state.
    pub fn current_action(&self, state: &OperationState) -> Option<BotCommand> {
        match state {
            OperationState::MovingToTarget => Some(BotCommand::MoveTo(self.target)),
            OperationState::EquippingTool => Some(BotCommand::EquipTool(self.tool)),
            OperationState::ExecutingAction => Some(BotCommand::BreakBlock(self.target)),
            _ => None,
        }
    }
}

// ═══════════════════════════════════════════════════════════════
// PlaceBlockOperation
// ═══════════════════════════════════════════════════════════════

/// State machine for placing a single block.
///
/// Lifecycle:
/// Idle → EquippingTool → MovingToTarget → ExecutingAction → Completed
pub struct PlaceBlockOperation {
    pub target: BlockPos,
    pub block_type: String,
    pub tool: ToolType,
}

impl PlaceBlockOperation {
    pub fn new(target: BlockPos, block_type: String, tool: ToolType) -> Self {
        Self {
            target,
            block_type,
            tool,
        }
    }

    pub fn advance(&self, state: OperationState, event: OperationEvent) -> OperationState {
        match (&state, &event) {
            // Happy path
            (OperationState::Idle, OperationEvent::Start) => OperationState::EquippingTool,
            (OperationState::EquippingTool, OperationEvent::ToolEquipped) => {
                OperationState::MovingToTarget
            }
            (OperationState::MovingToTarget, OperationEvent::Arrived) => {
                OperationState::ExecutingAction
            }
            (OperationState::ExecutingAction, OperationEvent::BlockPlaced) => {
                OperationState::Completed
            }

            // Failure from any state
            (_, OperationEvent::Failed(err)) => OperationState::Failed(err.clone()),

            // Terminal states are sticky
            (OperationState::Completed | OperationState::Failed(_), _) => state,

            // Invalid transition — stay in current state
            _ => state,
        }
    }

    pub fn current_action(&self, state: &OperationState) -> Option<BotCommand> {
        match state {
            OperationState::EquippingTool => Some(BotCommand::EquipTool(self.tool)),
            OperationState::MovingToTarget => Some(BotCommand::MoveTo(self.target)),
            OperationState::ExecutingAction => {
                Some(BotCommand::PlaceBlock(self.target, self.block_type.clone()))
            }
            _ => None,
        }
    }
}

// ═══════════════════════════════════════════════════════════════
// OpenContainerOperation
// ═══════════════════════════════════════════════════════════════

/// State machine for opening a container (chest, furnace, etc.).
///
/// Lifecycle:
/// Idle → MovingToTarget → ExecutingAction → Completed
pub struct OpenContainerOperation {
    pub target: BlockPos,
}

impl OpenContainerOperation {
    pub fn new(target: BlockPos) -> Self {
        Self { target }
    }

    pub fn advance(&self, state: OperationState, event: OperationEvent) -> OperationState {
        match (&state, &event) {
            // Happy path
            (OperationState::Idle, OperationEvent::Start) => OperationState::MovingToTarget,
            (OperationState::MovingToTarget, OperationEvent::Arrived) => {
                OperationState::ExecutingAction
            }
            (OperationState::ExecutingAction, OperationEvent::ContainerOpened) => {
                OperationState::Completed
            }

            // Failure from any state
            (_, OperationEvent::Failed(err)) => OperationState::Failed(err.clone()),

            // Terminal states are sticky
            (OperationState::Completed | OperationState::Failed(_), _) => state,

            // Invalid transition — stay in current state
            _ => state,
        }
    }

    pub fn current_action(&self, state: &OperationState) -> Option<BotCommand> {
        match state {
            OperationState::MovingToTarget => Some(BotCommand::MoveTo(self.target)),
            OperationState::ExecutingAction => Some(BotCommand::OpenContainer(self.target)),
            _ => None,
        }
    }
}

// ═══════════════════════════════════════════════════════════════
// EquipToolOperation
// ═══════════════════════════════════════════════════════════════

/// State machine for equipping a specific tool.
///
/// Lifecycle:
/// Idle → EquippingTool → Completed
pub struct EquipToolOperation {
    pub tool: ToolType,
}

impl EquipToolOperation {
    pub fn new(tool: ToolType) -> Self {
        Self { tool }
    }

    pub fn advance(&self, state: OperationState, event: OperationEvent) -> OperationState {
        match (&state, &event) {
            // Happy path
            (OperationState::Idle, OperationEvent::Start) => OperationState::EquippingTool,
            (OperationState::EquippingTool, OperationEvent::ToolEquipped) => {
                OperationState::Completed
            }

            // Failure from any state
            (_, OperationEvent::Failed(err)) => OperationState::Failed(err.clone()),

            // Terminal states are sticky
            (OperationState::Completed | OperationState::Failed(_), _) => state,

            // Invalid transition — stay in current state
            _ => state,
        }
    }

    pub fn current_action(&self, state: &OperationState) -> Option<BotCommand> {
        match state {
            OperationState::EquippingTool => Some(BotCommand::EquipTool(self.tool)),
            _ => None,
        }
    }
}

// ═══════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    // ── Helpers ─────────────────────────────────────────────

    fn test_pos() -> BlockPos {
        BlockPos::new(10, 64, 20)
    }

    fn test_err() -> BotError {
        BotError::Internal("test failure".into())
    }

    /// Build a `WorldSnapshot` from a block list, populating `block_index`
    /// the same way `SnapshotBuilder::build` does in production. Use this
    /// in tests instead of `WorldSnapshot { blocks, ..Default::default() }`
    /// when the test exercises code that looks up blocks via `block_index`
    /// (e.g. `find_standable_neighbor`, `find_obstacle_block`).
    fn make_snapshot_with_blocks(
        blocks: Vec<crate::types::BlockEntry>,
    ) -> crate::types::WorldSnapshot {
        use std::collections::HashMap;
        let block_index: HashMap<BlockPos, usize> = blocks
            .iter()
            .enumerate()
            .map(|(i, b)| (b.position, i))
            .collect();
        crate::types::WorldSnapshot {
            blocks,
            block_index,
            ..Default::default()
        }
    }

    // ── OperationState variant tests ────────────────────────

    #[test]
    fn test_operation_state_variants() {
        let states = [
            OperationState::Idle,
            OperationState::MovingToTarget,
            OperationState::EquippingTool,
            OperationState::ExecutingAction,
            OperationState::WaitingForResult,
            OperationState::Completed,
            OperationState::Failed(test_err()),
        ];
        assert_eq!(states.len(), 7);
    }

    #[test]
    fn test_operation_state_failed_contains_error() {
        let err = BotError::MiningInterrupted {
            reason: "creeper".into(),
        };
        let state = OperationState::Failed(err.clone());
        assert_eq!(state, OperationState::Failed(err));
    }

    #[test]
    fn test_operation_state_clone() {
        let s = OperationState::MovingToTarget;
        assert_eq!(s.clone(), s);
    }

    // ── OperationEvent variant tests ────────────────────────

    #[test]
    fn test_operation_event_variants() {
        let events = [
            OperationEvent::Start,
            OperationEvent::Arrived,
            OperationEvent::ToolEquipped,
            OperationEvent::ToolAlreadyInInventory,
            OperationEvent::ActionStarted,
            OperationEvent::BlockBroken,
            OperationEvent::BlockPlaced,
            OperationEvent::ContainerOpened,
            OperationEvent::Failed(test_err()),
        ];
        assert_eq!(events.len(), 9);
    }

    #[test]
    fn test_operation_event_clone() {
        let e = OperationEvent::Arrived;
        assert_eq!(e.clone(), e);
    }

    // ── find_standable_neighbor ─────────────────────────────

    #[test]
    fn test_find_standable_neighbor() {
        use crate::types::BlockEntry;

        // Snapshot:
        //   - target block (stone) at (0, 64, 0)
        //   - air at (1, 64, 0) — standable position
        //   - stone below at (1, 63, 0) — solid floor
        // Expected: returns (1, 64, 0)
        let target = BlockPos::new(0, 64, 0);
        let snapshot = make_snapshot_with_blocks(vec![
            BlockEntry {
                position: target,
                block_type: "stone".into(),
                block_state: None,
            },
            BlockEntry {
                position: BlockPos::new(1, 64, 0),
                block_type: "air".into(),
                block_state: None,
            },
            BlockEntry {
                position: BlockPos::new(1, 63, 0),
                block_type: "stone".into(),
                block_state: None,
            },
        ]);

        assert_eq!(
            find_standable_neighbor(&snapshot, target),
            Some(BlockPos::new(1, 64, 0))
        );
    }

    #[test]
    fn test_find_standable_neighbor_returns_none_when_no_floor() {
        use crate::types::BlockEntry;

        // Target at (0, 64, 0) with no floor below any neighbour —
        // nothing to stand on.
        let target = BlockPos::new(0, 64, 0);
        let snapshot = make_snapshot_with_blocks(vec![BlockEntry {
            position: target,
            block_type: "stone".into(),
            block_state: None,
        }]);

        assert_eq!(find_standable_neighbor(&snapshot, target), None);
    }

    #[test]
    fn test_find_standable_neighbor_skips_solid_neighbour() {
        use crate::types::BlockEntry;

        // Target at (0, 64, 0). Neighbour (1, 64, 0) is solid stone (not
        // air), so it's not standable even though (1, 63, 0) is solid floor.
        // Neighbour (-1, 64, 0) is air with stone below — standable.
        let target = BlockPos::new(0, 64, 0);
        let snapshot = make_snapshot_with_blocks(vec![
            BlockEntry {
                position: target,
                block_type: "stone".into(),
                block_state: None,
            },
            BlockEntry {
                position: BlockPos::new(1, 64, 0),
                block_type: "stone".into(),
                block_state: None,
            },
            BlockEntry {
                position: BlockPos::new(1, 63, 0),
                block_type: "stone".into(),
                block_state: None,
            },
            BlockEntry {
                position: BlockPos::new(-1, 64, 0),
                block_type: "air".into(),
                block_state: None,
            },
            BlockEntry {
                position: BlockPos::new(-1, 63, 0),
                block_type: "stone".into(),
                block_state: None,
            },
        ]);

        assert_eq!(
            find_standable_neighbor(&snapshot, target),
            Some(BlockPos::new(-1, 64, 0))
        );
    }

    #[test]
    fn test_find_standable_neighbor_uses_air_absent_from_snapshot() {
        use crate::types::BlockEntry;

        // Target at (0, 64, 0). Neighbour (1, 64, 0) is absent from the
        // snapshot (treated as air). Block below at (1, 63, 0) is stone.
        // Expected: returns (1, 64, 0).
        let target = BlockPos::new(0, 64, 0);
        let snapshot = make_snapshot_with_blocks(vec![
            BlockEntry {
                position: target,
                block_type: "stone".into(),
                block_state: None,
            },
            BlockEntry {
                position: BlockPos::new(1, 63, 0),
                block_type: "stone".into(),
                block_state: None,
            },
        ]);

        assert_eq!(
            find_standable_neighbor(&snapshot, target),
            Some(BlockPos::new(1, 64, 0))
        );
    }

    /// The 4 orthogonal neighbours are all solid; only the SE diagonal is
    /// air with a solid floor. The function must return the diagonal cell
    /// (1, 64, 1) — a case the old 4-direction scan would have missed.
    #[test]
    fn test_find_standable_neighbor_8_directions() {
        use crate::types::BlockEntry;

        let target = BlockPos::new(0, 64, 0);

        let mut blocks = vec![BlockEntry {
            position: target,
            block_type: "stone".into(),
            block_state: None,
        }];

        // Fill all 4 orthogonal neighbours with solid stone (so the old
        // 4-direction scan would find nothing).
        for (dx, dz) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
            blocks.push(BlockEntry {
                position: BlockPos::new(target.x + dx, target.y, target.z + dz),
                block_type: "stone".into(),
                block_state: None,
            });
            blocks.push(BlockEntry {
                position: BlockPos::new(target.x + dx, target.y - 1, target.z + dz),
                block_type: "stone".into(),
                block_state: None,
            });
        }

        // The SE diagonal (1, 64, 1) is air with stone below.
        blocks.push(BlockEntry {
            position: BlockPos::new(1, 64, 1),
            block_type: "air".into(),
            block_state: None,
        });
        blocks.push(BlockEntry {
            position: BlockPos::new(1, 63, 1),
            block_type: "stone".into(),
            block_state: None,
        });

        let snapshot = make_snapshot_with_blocks(blocks);
        assert_eq!(
            find_standable_neighbor(&snapshot, target),
            Some(BlockPos::new(1, 64, 1))
        );
    }

    /// All 4 orthogonal neighbours are solid and all 4 diagonals are
    /// air — but only ONE diagonal has a solid block below it. The function
    /// must still find that diagonal.
    #[test]
    fn test_find_standable_neighbor_finds_only_standable_diagonal() {
        use crate::types::BlockEntry;

        let target = BlockPos::new(0, 64, 0);

        let mut blocks = vec![BlockEntry {
            position: target,
            block_type: "stone".into(),
            block_state: None,
        }];

        // All 4 orthogonal neighbours are solid (no standable cells there).
        for (dx, dz) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
            blocks.push(BlockEntry {
                position: BlockPos::new(target.x + dx, target.y, target.z + dz),
                block_type: "stone".into(),
                block_state: None,
            });
            blocks.push(BlockEntry {
                position: BlockPos::new(target.x + dx, target.y - 1, target.z + dz),
                block_type: "stone".into(),
                block_state: None,
            });
        }

        // 3 of 4 diagonals: air but no floor → not standable.
        for (dx, dz) in [(-1, -1), (-1, 1), (1, -1)] {
            blocks.push(BlockEntry {
                position: BlockPos::new(target.x + dx, target.y, target.z + dz),
                block_type: "air".into(),
                block_state: None,
            });
            // Below these: also air (no floor).
            blocks.push(BlockEntry {
                position: BlockPos::new(target.x + dx, target.y - 1, target.z + dz),
                block_type: "air".into(),
                block_state: None,
            });
        }

        // 4th diagonal: air with stone floor → the only standable cell.
        blocks.push(BlockEntry {
            position: BlockPos::new(1, 64, 1),
            block_type: "air".into(),
            block_state: None,
        });
        blocks.push(BlockEntry {
            position: BlockPos::new(1, 63, 1),
            block_type: "stone".into(),
            block_state: None,
        });

        let snapshot = make_snapshot_with_blocks(blocks);
        assert_eq!(
            find_standable_neighbor(&snapshot, target),
            Some(BlockPos::new(1, 64, 1))
        );
    }

    /// All 8 directions scanned; only y+1 yields a standable cell. The Y
    /// priority must reach the upper level after exhausting y=0.
    #[test]
    fn test_find_standable_neighbor_y_priority_preserved() {
        use crate::types::BlockEntry;

        // Target at (0, 64, 0). All same-Y neighbours are solid (including
        // diagonals). The NW corner at y+1 is air with stone below.
        let target = BlockPos::new(0, 64, 0);
        let mut blocks = vec![BlockEntry {
            position: target,
            block_type: "stone".into(),
            block_state: None,
        }];

        for &(dx, dz) in &STANDABLE_OFFSETS_XZ {
            // Solid at the same Y — not standable.
            blocks.push(BlockEntry {
                position: BlockPos::new(target.x + dx, target.y, target.z + dz),
                block_type: "stone".into(),
                block_state: None,
            });
        }

        // At y+1 (65): NW diagonal (-1, 65, -1) is air with stone below.
        blocks.push(BlockEntry {
            position: BlockPos::new(target.x - 1, target.y + 1, target.z - 1),
            block_type: "air".into(),
            block_state: None,
        });
        blocks.push(BlockEntry {
            position: BlockPos::new(target.x - 1, target.y, target.z - 1),
            block_type: "stone".into(),
            block_state: None,
        });

        let snapshot = make_snapshot_with_blocks(blocks);
        assert_eq!(
            find_standable_neighbor(&snapshot, target),
            Some(BlockPos::new(target.x - 1, target.y + 1, target.z - 1))
        );
    }

    /// When no neighbour (orthogonal or diagonal, at any Y) is standable,
    /// the function returns `None` instead of panicking.
    #[test]
    fn test_find_standable_neighbor_returns_none_when_no_candidate() {
        use crate::types::BlockEntry;

        let target = BlockPos::new(0, 64, 0);

        // Fill the entire 3×3×3 area around the target with stone — no
        // standable cell exists anywhere.
        let mut blocks = vec![BlockEntry {
            position: target,
            block_type: "stone".into(),
            block_state: None,
        }];
        for dy in -1..=1 {
            for (dx, dz) in STANDABLE_OFFSETS_XZ.iter().copied() {
                blocks.push(BlockEntry {
                    position: BlockPos::new(target.x + dx, target.y + dy, target.z + dz),
                    block_type: "stone".into(),
                    block_state: None,
                });
            }
        }

        let snapshot = make_snapshot_with_blocks(blocks);
        assert_eq!(find_standable_neighbor(&snapshot, target), None);
    }

    /// Empty snapshot — no blocks at all — should return `None` cleanly
    /// (every cell is "absent", treated as air, but no floor exists either).
    #[test]
    fn test_find_standable_neighbor_empty_snapshot() {
        let target = BlockPos::new(0, 64, 0);
        let snapshot = make_snapshot_with_blocks(vec![]);
        assert_eq!(find_standable_neighbor(&snapshot, target), None);
    }

    // ── proptest ────────────────────────────────────────────────

    use proptest::prelude::*;

    proptest! {
        /// A standable-neighbour is always within 1 block of the target on
        /// both the X and Z axes, and within 1 block on the Y axis.
        /// Without this invariant, the bot would pathfind to far-away cells.
        #[test]
        fn prop_standable_neighbor_within_unit_radius(
            tx in -1000i32..1000,
            ty in -1000i32..1000,
            tz in -1000i32..1000,
            seed in any::<u64>(),
        ) {
            use crate::types::BlockEntry;
            let target = BlockPos::new(tx, ty, tz);
            // Deterministic but varied layout: place "stone" at every cell
            // (x, y, z) where hash(x, y, z) % 3 == 0, otherwise leave air.
            let mut blocks = vec![BlockEntry {
                position: target,
                block_type: "stone".into(),
                block_state: None,
            }];
            for &(dx, dz) in &STANDABLE_OFFSETS_XZ {
                for &dy in &[-1, 0, 1] {
                    let pos = BlockPos::new(target.x + dx, target.y + dy, target.z + dz);
                    let is_solid = (seed
                        .wrapping_add((pos.x as u64).wrapping_mul(2654435761))
                        .wrapping_add((pos.y as u64).wrapping_mul(40503))
                        .wrapping_add((pos.z as u64).wrapping_mul(16777619)))
                        % 3
                        == 0;
                    blocks.push(BlockEntry {
                        position: pos,
                        block_type: if is_solid { "stone" } else { "air" }.into(),
                        block_state: None,
                    });
                    if !is_solid {
                        // Floor at y-1: always present so the cell is standable.
                        let below = BlockPos::new(pos.x, pos.y - 1, pos.z);
                        if !blocks.iter().any(|b| b.position == below) {
                            blocks.push(BlockEntry {
                                position: below,
                                block_type: "stone".into(),
                                block_state: None,
                            });
                        }
                    }
                }
            }
            let snapshot = make_snapshot_with_blocks(blocks);
            if let Some(p) = find_standable_neighbor(&snapshot, target) {
                prop_assert!((p.x - target.x).abs() <= 1);
                prop_assert!((p.y - target.y).abs() <= 1);
                prop_assert!((p.z - target.z).abs() <= 1);
            }
        }

        /// Y priority: a standable cell at the same Y as the target is
        /// always preferred over one at y+1 or y-1.
        #[test]
        fn prop_standable_neighbor_y_priority(
            tx in -100i32..100,
            ty in 0i32..200,
            tz in -100i32..100,
        ) {
            use crate::types::BlockEntry;
            let target = BlockPos::new(tx, ty, tz);
            // Only y+1 has a standable cell; the same-Y ring is solid stone.
            let mut blocks = vec![BlockEntry {
                position: target,
                block_type: "stone".into(),
                block_state: None,
            }];
            for &(dx, dz) in &STANDABLE_OFFSETS_XZ {
                // Same-Y ring: solid (not standable).
                blocks.push(BlockEntry {
                    position: BlockPos::new(target.x + dx, target.y, target.z + dz),
                    block_type: "stone".into(),
                    block_state: None,
                });
                // y+1 cell: air with stone floor (standable).
                blocks.push(BlockEntry {
                    position: BlockPos::new(target.x + dx, target.y + 1, target.z + dz),
                    block_type: "air".into(),
                    block_state: None,
                });
                blocks.push(BlockEntry {
                    position: BlockPos::new(target.x + dx, target.y, target.z + dz),
                    block_type: "stone".into(),
                    block_state: None,
                });
            }
            let snapshot = make_snapshot_with_blocks(blocks);
            let found = find_standable_neighbor(&snapshot, target);
            prop_assert!(found.is_some(), "should find a y+1 standable cell");
            // The Y priority dictates y=0 first, but here y=0 is fully solid.
            // So we should fall through to y+1.
            let pos = found.unwrap();
            prop_assert!(pos.y > target.y, "y+1 is the only standable ring, got y={}", pos.y);
        }

        /// An empty snapshot always returns `None` (no floor anywhere).
        #[test]
        fn prop_standable_neighbor_empty_snapshot(
            tx in -1000i32..1000,
            ty in -1000i32..1000,
            tz in -1000i32..1000,
        ) {
            let target = BlockPos::new(tx, ty, tz);
            let snapshot = make_snapshot_with_blocks(vec![]);
            prop_assert_eq!(find_standable_neighbor(&snapshot, target), None);
        }

        /// When all 8 horizontal neighbours at all 3 Y levels are solid
        /// stone, the function must return `None` rather than panicking.
        #[test]
        fn prop_standable_neighbor_buried(
            tx in -100i32..100,
            ty in 0i32..200,
            tz in -100i32..100,
        ) {
            use crate::types::BlockEntry;
            let target = BlockPos::new(tx, ty, tz);
            let mut blocks = vec![BlockEntry {
                position: target,
                block_type: "stone".into(),
                block_state: None,
            }];
            for &dy in &[-1, 0, 1] {
                for &(dx, dz) in &STANDABLE_OFFSETS_XZ {
                    blocks.push(BlockEntry {
                        position: BlockPos::new(target.x + dx, target.y + dy, target.z + dz),
                        block_type: "stone".into(),
                        block_state: None,
                    });
                }
            }
            let snapshot = make_snapshot_with_blocks(blocks);
            prop_assert_eq!(find_standable_neighbor(&snapshot, target), None);
        }
    }

    // ── MineBlockOperation: happy path ──────────────────────

    #[test]
    fn test_mine_block_happy_path() {
        let op = MineBlockOperation::new(test_pos(), ToolType::Pickaxe);
        let mut state = OperationState::Idle;

        state = op.advance(state, OperationEvent::Start);
        assert_eq!(state, OperationState::MovingToTarget);

        state = op.advance(state, OperationEvent::Arrived);
        assert_eq!(state, OperationState::EquippingTool);

        state = op.advance(state, OperationEvent::ToolEquipped);
        assert_eq!(state, OperationState::ExecutingAction);

        state = op.advance(state, OperationEvent::ActionStarted);
        assert_eq!(state, OperationState::WaitingForResult);

        state = op.advance(state, OperationEvent::BlockBroken);
        assert_eq!(state, OperationState::Completed);
    }

    #[test]
    fn test_mine_block_current_actions() {
        let op = MineBlockOperation::new(test_pos(), ToolType::Pickaxe);

        assert_eq!(
            op.current_action(&OperationState::MovingToTarget),
            Some(BotCommand::MoveTo(test_pos()))
        );
        assert_eq!(
            op.current_action(&OperationState::EquippingTool),
            Some(BotCommand::EquipTool(ToolType::Pickaxe))
        );
        assert_eq!(
            op.current_action(&OperationState::ExecutingAction),
            Some(BotCommand::BreakBlock(test_pos()))
        );
        assert_eq!(op.current_action(&OperationState::WaitingForResult), None);
        assert_eq!(op.current_action(&OperationState::Completed), None);
        assert_eq!(op.current_action(&OperationState::Idle), None);
    }

    // ── MineBlockOperation: failure handling ─────────────────

    #[test]
    fn test_mine_block_fails_from_idle() {
        let op = MineBlockOperation::new(test_pos(), ToolType::Pickaxe);
        let err = test_err();
        let state = op.advance(OperationState::Idle, OperationEvent::Failed(err.clone()));
        assert_eq!(state, OperationState::Failed(err));
    }

    #[test]
    fn test_mine_block_fails_from_moving() {
        let op = MineBlockOperation::new(test_pos(), ToolType::Pickaxe);
        let err = test_err();
        let state = op.advance(
            OperationState::MovingToTarget,
            OperationEvent::Failed(err.clone()),
        );
        assert_eq!(state, OperationState::Failed(err));
    }

    #[test]
    fn test_mine_block_fails_from_equipping() {
        let op = MineBlockOperation::new(test_pos(), ToolType::Pickaxe);
        let err = test_err();
        let state = op.advance(
            OperationState::EquippingTool,
            OperationEvent::Failed(err.clone()),
        );
        assert_eq!(state, OperationState::Failed(err));
    }

    #[test]
    fn test_mine_block_fails_from_executing() {
        let op = MineBlockOperation::new(test_pos(), ToolType::Pickaxe);
        let err = test_err();
        let state = op.advance(
            OperationState::ExecutingAction,
            OperationEvent::Failed(err.clone()),
        );
        assert_eq!(state, OperationState::Failed(err));
    }

    #[test]
    fn test_mine_block_fails_from_waiting() {
        let op = MineBlockOperation::new(test_pos(), ToolType::Pickaxe);
        let err = test_err();
        let state = op.advance(
            OperationState::WaitingForResult,
            OperationEvent::Failed(err.clone()),
        );
        assert_eq!(state, OperationState::Failed(err));
    }

    #[test]
    fn test_mine_block_completed_is_sticky() {
        let op = MineBlockOperation::new(test_pos(), ToolType::Pickaxe);
        let state = op.advance(OperationState::Completed, OperationEvent::Start);
        assert_eq!(state, OperationState::Completed);
    }

    #[test]
    fn test_mine_block_failed_is_sticky() {
        let op = MineBlockOperation::new(test_pos(), ToolType::Pickaxe);
        let err = test_err();
        let failed = OperationState::Failed(err);
        let state = op.advance(failed.clone(), OperationEvent::Start);
        assert_eq!(state, failed);
    }

    #[test]
    fn test_mine_block_invalid_transition_stays() {
        let op = MineBlockOperation::new(test_pos(), ToolType::Pickaxe);
        let state = op.advance(OperationState::Idle, OperationEvent::Arrived);
        assert_eq!(state, OperationState::Idle);
    }

    // ── MineBlockOperation: ToolAlreadyInInventory skip (Task 1.5) ─

    #[test]
    fn test_mine_block_skip_equip_when_tool_in_inventory() {
        // Task 1.5 (P0-#3): when the best tool is already in the bot's
        // inventory (but cannot be auto-equipped), the state machine
        // should transition from `MovingToTarget` to `ExecutingAction`
        // via the new `ToolAlreadyInInventory` event — skipping the
        // `EquippingTool` step entirely.
        let op = MineBlockOperation::new(test_pos(), ToolType::Pickaxe);
        let mut state = OperationState::Idle;

        state = op.advance(state, OperationEvent::Start);
        assert_eq!(state, OperationState::MovingToTarget);

        // Tool is in inventory but cannot be hotbar-switched: skip equip.
        state = op.advance(state, OperationEvent::ToolAlreadyInInventory);
        assert_eq!(state, OperationState::ExecutingAction);

        // Continue with the rest of the happy path.
        state = op.advance(state, OperationEvent::ActionStarted);
        assert_eq!(state, OperationState::WaitingForResult);

        state = op.advance(state, OperationEvent::BlockBroken);
        assert_eq!(state, OperationState::Completed);
    }

    #[test]
    fn test_mine_block_tool_already_inventory_only_valid_from_moving() {
        // `ToolAlreadyInInventory` should only be valid from `MovingToTarget`.
        // From any other state, the transition is invalid (state stays put).
        let op = MineBlockOperation::new(test_pos(), ToolType::Pickaxe);

        // Idle: invalid, stays Idle.
        let state = op.advance(OperationState::Idle, OperationEvent::ToolAlreadyInInventory);
        assert_eq!(state, OperationState::Idle);

        // EquippingTool: invalid, stays put.
        let state = op.advance(
            OperationState::EquippingTool,
            OperationEvent::ToolAlreadyInInventory,
        );
        assert_eq!(state, OperationState::EquippingTool);

        // ExecutingAction: invalid, stays put.
        let state = op.advance(
            OperationState::ExecutingAction,
            OperationEvent::ToolAlreadyInInventory,
        );
        assert_eq!(state, OperationState::ExecutingAction);

        // Failed: sticky.
        let failed = OperationState::Failed(test_err());
        let state = op.advance(failed.clone(), OperationEvent::ToolAlreadyInInventory);
        assert_eq!(state, failed);
    }

    // ── PlaceBlockOperation: happy path ─────────────────────

    #[test]
    fn test_place_block_happy_path() {
        let op = PlaceBlockOperation::new(test_pos(), "stone".into(), ToolType::Hand);
        let mut state = OperationState::Idle;

        state = op.advance(state, OperationEvent::Start);
        assert_eq!(state, OperationState::EquippingTool);

        state = op.advance(state, OperationEvent::ToolEquipped);
        assert_eq!(state, OperationState::MovingToTarget);

        state = op.advance(state, OperationEvent::Arrived);
        assert_eq!(state, OperationState::ExecutingAction);

        state = op.advance(state, OperationEvent::BlockPlaced);
        assert_eq!(state, OperationState::Completed);
    }

    #[test]
    fn test_place_block_current_actions() {
        let op = PlaceBlockOperation::new(test_pos(), "dirt".into(), ToolType::Hand);

        assert_eq!(
            op.current_action(&OperationState::EquippingTool),
            Some(BotCommand::EquipTool(ToolType::Hand))
        );
        assert_eq!(
            op.current_action(&OperationState::MovingToTarget),
            Some(BotCommand::MoveTo(test_pos()))
        );
        assert_eq!(
            op.current_action(&OperationState::ExecutingAction),
            Some(BotCommand::PlaceBlock(test_pos(), "dirt".into()))
        );
        assert_eq!(op.current_action(&OperationState::Idle), None);
        assert_eq!(op.current_action(&OperationState::Completed), None);
    }

    // ── PlaceBlockOperation: failure handling ────────────────

    #[test]
    fn test_place_block_fails_from_idle() {
        let op = PlaceBlockOperation::new(test_pos(), "stone".into(), ToolType::Hand);
        let err = test_err();
        let state = op.advance(OperationState::Idle, OperationEvent::Failed(err.clone()));
        assert_eq!(state, OperationState::Failed(err));
    }

    #[test]
    fn test_place_block_fails_from_equipping() {
        let op = PlaceBlockOperation::new(test_pos(), "stone".into(), ToolType::Hand);
        let err = test_err();
        let state = op.advance(
            OperationState::EquippingTool,
            OperationEvent::Failed(err.clone()),
        );
        assert_eq!(state, OperationState::Failed(err));
    }

    #[test]
    fn test_place_block_fails_from_moving() {
        let op = PlaceBlockOperation::new(test_pos(), "stone".into(), ToolType::Hand);
        let err = test_err();
        let state = op.advance(
            OperationState::MovingToTarget,
            OperationEvent::Failed(err.clone()),
        );
        assert_eq!(state, OperationState::Failed(err));
    }

    #[test]
    fn test_place_block_fails_from_executing() {
        let op = PlaceBlockOperation::new(test_pos(), "stone".into(), ToolType::Hand);
        let err = test_err();
        let state = op.advance(
            OperationState::ExecutingAction,
            OperationEvent::Failed(err.clone()),
        );
        assert_eq!(state, OperationState::Failed(err));
    }

    #[test]
    fn test_place_block_completed_is_sticky() {
        let op = PlaceBlockOperation::new(test_pos(), "stone".into(), ToolType::Hand);
        let state = op.advance(OperationState::Completed, OperationEvent::Start);
        assert_eq!(state, OperationState::Completed);
    }

    #[test]
    fn test_place_block_failed_is_sticky() {
        let op = PlaceBlockOperation::new(test_pos(), "stone".into(), ToolType::Hand);
        let err = test_err();
        let failed = OperationState::Failed(err);
        let state = op.advance(failed.clone(), OperationEvent::Start);
        assert_eq!(state, failed);
    }

    #[test]
    fn test_place_block_invalid_transition_stays() {
        let op = PlaceBlockOperation::new(test_pos(), "stone".into(), ToolType::Hand);
        let state = op.advance(OperationState::Idle, OperationEvent::Arrived);
        assert_eq!(state, OperationState::Idle);
    }

    // ── OpenContainerOperation: happy path ──────────────────

    #[test]
    fn test_open_container_happy_path() {
        let op = OpenContainerOperation::new(test_pos());
        let mut state = OperationState::Idle;

        state = op.advance(state, OperationEvent::Start);
        assert_eq!(state, OperationState::MovingToTarget);

        state = op.advance(state, OperationEvent::Arrived);
        assert_eq!(state, OperationState::ExecutingAction);

        state = op.advance(state, OperationEvent::ContainerOpened);
        assert_eq!(state, OperationState::Completed);
    }

    #[test]
    fn test_open_container_current_actions() {
        let op = OpenContainerOperation::new(test_pos());

        assert_eq!(
            op.current_action(&OperationState::MovingToTarget),
            Some(BotCommand::MoveTo(test_pos()))
        );
        assert_eq!(
            op.current_action(&OperationState::ExecutingAction),
            Some(BotCommand::OpenContainer(test_pos()))
        );
        assert_eq!(op.current_action(&OperationState::Idle), None);
        assert_eq!(op.current_action(&OperationState::Completed), None);
    }

    // ── OpenContainerOperation: failure handling ─────────────

    #[test]
    fn test_open_container_fails_from_idle() {
        let op = OpenContainerOperation::new(test_pos());
        let err = test_err();
        let state = op.advance(OperationState::Idle, OperationEvent::Failed(err.clone()));
        assert_eq!(state, OperationState::Failed(err));
    }

    #[test]
    fn test_open_container_fails_from_moving() {
        let op = OpenContainerOperation::new(test_pos());
        let err = test_err();
        let state = op.advance(
            OperationState::MovingToTarget,
            OperationEvent::Failed(err.clone()),
        );
        assert_eq!(state, OperationState::Failed(err));
    }

    #[test]
    fn test_open_container_fails_from_executing() {
        let op = OpenContainerOperation::new(test_pos());
        let err = test_err();
        let state = op.advance(
            OperationState::ExecutingAction,
            OperationEvent::Failed(err.clone()),
        );
        assert_eq!(state, OperationState::Failed(err));
    }

    #[test]
    fn test_open_container_completed_is_sticky() {
        let op = OpenContainerOperation::new(test_pos());
        let state = op.advance(OperationState::Completed, OperationEvent::Start);
        assert_eq!(state, OperationState::Completed);
    }

    #[test]
    fn test_open_container_failed_is_sticky() {
        let op = OpenContainerOperation::new(test_pos());
        let err = test_err();
        let failed = OperationState::Failed(err);
        let state = op.advance(failed.clone(), OperationEvent::Start);
        assert_eq!(state, failed);
    }

    #[test]
    fn test_open_container_invalid_transition_stays() {
        let op = OpenContainerOperation::new(test_pos());
        let state = op.advance(OperationState::Idle, OperationEvent::ContainerOpened);
        assert_eq!(state, OperationState::Idle);
    }

    // ── EquipToolOperation: happy path ──────────────────────

    #[test]
    fn test_equip_tool_happy_path() {
        let op = EquipToolOperation::new(ToolType::Axe);
        let mut state = OperationState::Idle;

        state = op.advance(state, OperationEvent::Start);
        assert_eq!(state, OperationState::EquippingTool);

        state = op.advance(state, OperationEvent::ToolEquipped);
        assert_eq!(state, OperationState::Completed);
    }

    #[test]
    fn test_equip_tool_current_actions() {
        let op = EquipToolOperation::new(ToolType::Shovel);

        assert_eq!(
            op.current_action(&OperationState::EquippingTool),
            Some(BotCommand::EquipTool(ToolType::Shovel))
        );
        assert_eq!(op.current_action(&OperationState::Idle), None);
        assert_eq!(op.current_action(&OperationState::Completed), None);
    }

    // ── EquipToolOperation: failure handling ─────────────────

    #[test]
    fn test_equip_tool_fails_from_idle() {
        let op = EquipToolOperation::new(ToolType::Sword);
        let err = test_err();
        let state = op.advance(OperationState::Idle, OperationEvent::Failed(err.clone()));
        assert_eq!(state, OperationState::Failed(err));
    }

    #[test]
    fn test_equip_tool_fails_from_equipping() {
        let op = EquipToolOperation::new(ToolType::Sword);
        let err = test_err();
        let state = op.advance(
            OperationState::EquippingTool,
            OperationEvent::Failed(err.clone()),
        );
        assert_eq!(state, OperationState::Failed(err));
    }

    #[test]
    fn test_equip_tool_completed_is_sticky() {
        let op = EquipToolOperation::new(ToolType::Shears);
        let state = op.advance(OperationState::Completed, OperationEvent::Start);
        assert_eq!(state, OperationState::Completed);
    }

    #[test]
    fn test_equip_tool_failed_is_sticky() {
        let op = EquipToolOperation::new(ToolType::Hand);
        let err = test_err();
        let failed = OperationState::Failed(err);
        let state = op.advance(failed.clone(), OperationEvent::Start);
        assert_eq!(state, failed);
    }

    #[test]
    fn test_equip_tool_invalid_transition_stays() {
        let op = EquipToolOperation::new(ToolType::Pickaxe);
        let state = op.advance(OperationState::Idle, OperationEvent::Arrived);
        assert_eq!(state, OperationState::Idle);
    }

    // ── Cross-operation: different tools / positions ─────────

    #[test]
    fn test_mine_block_with_different_tools() {
        let pos = BlockPos::new(5, 5, 5);
        for tool in [ToolType::Pickaxe, ToolType::Axe, ToolType::Shovel] {
            let op = MineBlockOperation::new(pos, tool);
            assert_eq!(
                op.current_action(&OperationState::EquippingTool),
                Some(BotCommand::EquipTool(tool))
            );
        }
    }

    #[test]
    fn test_place_block_with_different_types() {
        let pos = BlockPos::new(1, 2, 3);
        let op = PlaceBlockOperation::new(pos, "oak_planks".into(), ToolType::Hand);
        assert_eq!(
            op.current_action(&OperationState::ExecutingAction),
            Some(BotCommand::PlaceBlock(pos, "oak_planks".into()))
        );
    }

    #[test]
    fn test_open_container_different_positions() {
        let pos = BlockPos::new(100, 64, -50);
        let op = OpenContainerOperation::new(pos);
        assert_eq!(
            op.current_action(&OperationState::MovingToTarget),
            Some(BotCommand::MoveTo(pos))
        );
    }

    // ── Exhaustive state coverage ─────────────────────────

    #[test]
    fn test_all_states_are_reachable_in_mine_block() {
        let op = MineBlockOperation::new(test_pos(), ToolType::Pickaxe);
        let mut state = OperationState::Idle;

        // Reach every state in the happy path
        state = op.advance(state, OperationEvent::Start);
        assert!(matches!(state, OperationState::MovingToTarget));

        state = op.advance(state, OperationEvent::Arrived);
        assert!(matches!(state, OperationState::EquippingTool));

        state = op.advance(state, OperationEvent::ToolEquipped);
        assert!(matches!(state, OperationState::ExecutingAction));

        state = op.advance(state, OperationEvent::ActionStarted);
        assert!(matches!(state, OperationState::WaitingForResult));

        state = op.advance(state, OperationEvent::BlockBroken);
        assert!(matches!(state, OperationState::Completed));
    }

    #[test]
    fn test_all_states_are_reachable_in_place_block() {
        let op = PlaceBlockOperation::new(test_pos(), "cobblestone".into(), ToolType::Hand);
        let mut state = OperationState::Idle;

        state = op.advance(state, OperationEvent::Start);
        assert!(matches!(state, OperationState::EquippingTool));

        state = op.advance(state, OperationEvent::ToolEquipped);
        assert!(matches!(state, OperationState::MovingToTarget));

        state = op.advance(state, OperationEvent::Arrived);
        assert!(matches!(state, OperationState::ExecutingAction));

        state = op.advance(state, OperationEvent::BlockPlaced);
        assert!(matches!(state, OperationState::Completed));
    }

    #[test]
    fn test_all_states_are_reachable_in_open_container() {
        let op = OpenContainerOperation::new(test_pos());
        let mut state = OperationState::Idle;

        state = op.advance(state, OperationEvent::Start);
        assert!(matches!(state, OperationState::MovingToTarget));

        state = op.advance(state, OperationEvent::Arrived);
        assert!(matches!(state, OperationState::ExecutingAction));

        state = op.advance(state, OperationEvent::ContainerOpened);
        assert!(matches!(state, OperationState::Completed));
    }

    #[test]
    fn test_all_states_are_reachable_in_equip_tool() {
        let op = EquipToolOperation::new(ToolType::Sword);
        let mut state = OperationState::Idle;

        state = op.advance(state, OperationEvent::Start);
        assert!(matches!(state, OperationState::EquippingTool));

        state = op.advance(state, OperationEvent::ToolEquipped);
        assert!(matches!(state, OperationState::Completed));
    }
}
