use crate::brain::modes::heating_mode::HeatingMode;

#[derive(Debug)]
pub enum Intention {
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
}