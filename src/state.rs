use crate::phase::Phase;
use crate::types::{Capabilities, Configuration, Decision};
use std::time::Instant;

#[derive(Debug, Clone, Copy)]
pub enum RecoveryState {
    Normal,
    Recovery { deadline: Instant },
    Probing { deadline: Instant },
}

#[derive(Debug)]
pub struct State {
    pub phase: Phase,
    pub config: Configuration,
    pub capabilities: Capabilities,

    pub calibrated: bool,
    pub last_decision: Option<Decision>,
}

impl State {
    pub fn new(config: Configuration, capabilities: Capabilities) -> Self {
        Self {
            phase: Phase::Startup,
            config,
            capabilities,
            calibrated: false,
            last_decision: None,
        }
    }

    pub fn transition(&mut self, phase: Phase) {
        self.phase = phase;
    }
}
