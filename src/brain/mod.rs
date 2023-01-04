use backtrace::Backtrace;
use tokio::runtime::Runtime;
use crate::io::IOBundle;

pub mod dummy;
pub mod python_like;

#[derive(Debug)]
pub struct BrainFailure {
    description: String,
    trace: Backtrace,
    actions: CorrectiveActions,
}

impl BrainFailure {
    pub fn new(description: String, actions: CorrectiveActions) -> Self {
        BrainFailure {
            description,
            trace: Backtrace::new(),
            actions,
        }
    }

    pub fn get_corrective_actions(&self) -> &CorrectiveActions {
        &self.actions
    }
}

#[derive(Debug)]
pub struct CorrectiveActions {
    heating_control_state_unknown: bool,
}

pub trait Brain {
    fn run(&mut self, runtime: &Runtime, io_bundle: &mut IOBundle) -> Result<(), BrainFailure>;

    fn reload_config(&mut self);
}

impl CorrectiveActions {

    pub fn new() -> Self {
        CorrectiveActions {
            heating_control_state_unknown: false,
        }
    }

    pub fn unknown_heating() -> Self {
        CorrectiveActions::new().with_unknown_heating_control_state()
    }

    pub fn is_heating_in_unknown_state(&self) -> bool {
        self.heating_control_state_unknown
    }

    pub fn with_unknown_heating_control_state(mut self) -> Self {
        self.heating_control_state_unknown = true;
        self
    }
}
