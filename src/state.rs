use crate::phase::Phase;
use crate::types::{Capabilities, Configuration, Decision};
#[derive(Debug, Clone, Copy)]
pub enum RecoveryState {
    Normal,
    Recovery { ticks: u32 },
    Probing,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryAction {
    None,
    Stop,
    Resume,
    Interrupt,
}
impl RecoveryState {
    pub fn tick(&mut self) -> RecoveryAction {
        match *self {
            RecoveryState::Recovery { ticks } if ticks >= 99 => {
                *self = RecoveryState::Probing;
                RecoveryAction::Resume
            }

            RecoveryState::Recovery { ticks } => {
                *self = RecoveryState::Recovery { ticks: ticks + 1 };
                RecoveryAction::None
            }

            _ => RecoveryAction::None,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionRequest {
    Continue,
    Pause,
    Interrupt,
    Halt,
}

#[derive(Debug)]
pub struct State {
    pub phase: Phase,
    pub config: Configuration,
    pub capabilities: Capabilities,
    pub calibrated: bool,
    pub last_decision: Option<Decision>,
    pub action_request: ActionRequest,
}

impl State {
    pub fn new(config: Configuration, capabilities: Capabilities) -> Self {
        Self {
            phase: Phase::Startup,
            config,
            capabilities,
            calibrated: false,
            last_decision: None,
            action_request: ActionRequest::Continue,
        }
    }

    pub fn transition(&mut self, phase: Phase) {
        self.phase = phase;
    }
}
