use crate::brain::modes::heating_mode::HeatingMode;

#[derive(Debug, PartialEq)]
pub enum Intention {
    SwitchForce(HeatingMode),
    KeepState,
    FinishMode,
}

impl Intention {
    /// Turn off immediately
    #[must_use]
    pub fn off_now() -> Intention {
        Intention::SwitchForce(HeatingMode::off())
    }

    /// Shows that this state has ended,
    /// and so another state must begin,
    /// if no state believes it should activate
    /// then this will turn everything off.
    #[must_use]
    pub fn finish() -> Intention {
        Intention::FinishMode
    }
}
