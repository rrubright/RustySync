#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Startup,

    Configured,
    Characterizing,
    Characterized,

    Calibrating,
    Calibrated,

    Running,
    Paused,
    Resuming,

    Completed,
    Shutdown,

    Error,
}
