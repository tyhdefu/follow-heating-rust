use crate::brain::python_like::modes::heating_mode::HeatingMode;

#[derive(Debug)]
pub enum Intention {
    /// Shows that the heating should
    /// switch its state to this state
    Change(ChangeState),
    SwitchForce(HeatingMode),
    KeepState,
    FinishMode,
}

impl Intention {
    /// Turn off immediately
    pub fn off_now() -> Intention {
        Intention::SwitchForce(HeatingMode::Off)
    }

    /// Shows that this state has ended,
    /// and so another state must begin,
    /// if no state believes it should activate
    /// then this will turn everything off.
    pub fn finish() -> Intention {
        Intention::FinishMode
    }

    /// Tells it to switch into the circulating mode.
    pub fn begin_circulating() -> Intention {
        Intention::Change(ChangeState::BeginCirculating)
    }
}

#[derive(Debug)]
pub enum ChangeState {
    BeginCirculating,
}
