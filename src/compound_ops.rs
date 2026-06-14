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
            (s, _) if matches!(s, OperationState::Completed | OperationState::Failed(_)) => state,

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
}

impl PlaceBlockOperation {
    pub fn new(target: BlockPos, block_type: String) -> Self {
        Self {
            target,
            block_type,
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
            (s, _) if matches!(s, OperationState::Completed | OperationState::Failed(_)) => state,

            // Invalid transition — stay in current state
            _ => state,
        }
    }

    pub fn current_action(&self, state: &OperationState) -> Option<BotCommand> {
        match state {
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
            (s, _) if matches!(s, OperationState::Completed | OperationState::Failed(_)) => state,

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
            (s, _) if matches!(s, OperationState::Completed | OperationState::Failed(_)) => state,

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

    // ── OperationState variant tests ────────────────────────

    #[test]
    fn test_operation_state_variants() {
        let states = vec![
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
        let events = vec![
            OperationEvent::Start,
            OperationEvent::Arrived,
            OperationEvent::ToolEquipped,
            OperationEvent::ActionStarted,
            OperationEvent::BlockBroken,
            OperationEvent::BlockPlaced,
            OperationEvent::ContainerOpened,
            OperationEvent::Failed(test_err()),
        ];
        assert_eq!(events.len(), 8);
    }

    #[test]
    fn test_operation_event_clone() {
        let e = OperationEvent::Arrived;
        assert_eq!(e.clone(), e);
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
        let state =
            op.advance(OperationState::MovingToTarget, OperationEvent::Failed(err.clone()));
        assert_eq!(state, OperationState::Failed(err));
    }

    #[test]
    fn test_mine_block_fails_from_equipping() {
        let op = MineBlockOperation::new(test_pos(), ToolType::Pickaxe);
        let err = test_err();
        let state =
            op.advance(OperationState::EquippingTool, OperationEvent::Failed(err.clone()));
        assert_eq!(state, OperationState::Failed(err));
    }

    #[test]
    fn test_mine_block_fails_from_executing() {
        let op = MineBlockOperation::new(test_pos(), ToolType::Pickaxe);
        let err = test_err();
        let state =
            op.advance(OperationState::ExecutingAction, OperationEvent::Failed(err.clone()));
        assert_eq!(state, OperationState::Failed(err));
    }

    #[test]
    fn test_mine_block_fails_from_waiting() {
        let op = MineBlockOperation::new(test_pos(), ToolType::Pickaxe);
        let err = test_err();
        let state =
            op.advance(OperationState::WaitingForResult, OperationEvent::Failed(err.clone()));
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

    // ── PlaceBlockOperation: happy path ─────────────────────

    #[test]
    fn test_place_block_happy_path() {
        let op = PlaceBlockOperation::new(test_pos(), "stone".into());
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
        let op = PlaceBlockOperation::new(test_pos(), "dirt".into());

        assert_eq!(
            op.current_action(&OperationState::MovingToTarget),
            Some(BotCommand::MoveTo(test_pos()))
        );
        assert_eq!(
            op.current_action(&OperationState::ExecutingAction),
            Some(BotCommand::PlaceBlock(test_pos(), "dirt".into()))
        );
        assert_eq!(op.current_action(&OperationState::EquippingTool), None);
        assert_eq!(op.current_action(&OperationState::Idle), None);
        assert_eq!(op.current_action(&OperationState::Completed), None);
    }

    // ── PlaceBlockOperation: failure handling ────────────────

    #[test]
    fn test_place_block_fails_from_idle() {
        let op = PlaceBlockOperation::new(test_pos(), "stone".into());
        let err = test_err();
        let state = op.advance(OperationState::Idle, OperationEvent::Failed(err.clone()));
        assert_eq!(state, OperationState::Failed(err));
    }

    #[test]
    fn test_place_block_fails_from_equipping() {
        let op = PlaceBlockOperation::new(test_pos(), "stone".into());
        let err = test_err();
        let state =
            op.advance(OperationState::EquippingTool, OperationEvent::Failed(err.clone()));
        assert_eq!(state, OperationState::Failed(err));
    }

    #[test]
    fn test_place_block_fails_from_moving() {
        let op = PlaceBlockOperation::new(test_pos(), "stone".into());
        let err = test_err();
        let state =
            op.advance(OperationState::MovingToTarget, OperationEvent::Failed(err.clone()));
        assert_eq!(state, OperationState::Failed(err));
    }

    #[test]
    fn test_place_block_fails_from_executing() {
        let op = PlaceBlockOperation::new(test_pos(), "stone".into());
        let err = test_err();
        let state =
            op.advance(OperationState::ExecutingAction, OperationEvent::Failed(err.clone()));
        assert_eq!(state, OperationState::Failed(err));
    }

    #[test]
    fn test_place_block_completed_is_sticky() {
        let op = PlaceBlockOperation::new(test_pos(), "stone".into());
        let state = op.advance(OperationState::Completed, OperationEvent::Start);
        assert_eq!(state, OperationState::Completed);
    }

    #[test]
    fn test_place_block_failed_is_sticky() {
        let op = PlaceBlockOperation::new(test_pos(), "stone".into());
        let err = test_err();
        let failed = OperationState::Failed(err);
        let state = op.advance(failed.clone(), OperationEvent::Start);
        assert_eq!(state, failed);
    }

    #[test]
    fn test_place_block_invalid_transition_stays() {
        let op = PlaceBlockOperation::new(test_pos(), "stone".into());
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
        let state =
            op.advance(OperationState::MovingToTarget, OperationEvent::Failed(err.clone()));
        assert_eq!(state, OperationState::Failed(err));
    }

    #[test]
    fn test_open_container_fails_from_executing() {
        let op = OpenContainerOperation::new(test_pos());
        let err = test_err();
        let state =
            op.advance(OperationState::ExecutingAction, OperationEvent::Failed(err.clone()));
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
        let state =
            op.advance(OperationState::EquippingTool, OperationEvent::Failed(err.clone()));
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
        let op = PlaceBlockOperation::new(pos, "oak_planks".into());
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
        let op = PlaceBlockOperation::new(test_pos(), "cobblestone".into());
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
