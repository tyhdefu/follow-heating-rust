use std::fmt::{Display, Formatter};
use backtrace::Backtrace;
use tokio::runtime::Runtime;
use crate::io::IOBundle;
use crate::time_util::mytime::TimeProvider;

pub mod dummy;
pub mod python_like;

mod boost_active_rooms;
mod modes;
mod immersion_heater;

#[derive(Debug)]
pub struct BrainFailure {
    description: String,
    trace: Backtrace,
    line_num: u32,
    file_name: String,
    actions: CorrectiveActions,
}

impl BrainFailure {
    pub fn new(description: String, trace: Backtrace, line_num: u32, file_name: String, actions: CorrectiveActions) -> Self {
        BrainFailure {
            description,
            trace,
            line_num,
            file_name,
            actions,
        }
    }

    pub fn get_corrective_actions(&self) -> &CorrectiveActions {
        &self.actions
    }
}

impl Display for BrainFailure {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "BrainFailure occured: '{}'\n", self.description)?;
        write!(f, "Recommended corrective actions: {:?}\n", self.actions)?;
        write!(f, "At: Line {} in {}\n", self.line_num, self.file_name)?;
        write!(f, "Trace:\n{:?}", self.trace)
    }
}

#[derive(Debug)]
pub struct CorrectiveActions {
    heating_control_state_unknown: bool,
}

pub trait Brain {
    fn run(&mut self, runtime: &Runtime, io_bundle: &mut IOBundle, time_provider: &impl TimeProvider) -> Result<(), BrainFailure>;

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

#[macro_export]
macro_rules! brain_fail {
    ($msg:expr) => {
        {
            let trace = backtrace::Backtrace::new();
            let actions = crate::brain::CorrectiveActions::new();
            BrainFailure::new($msg.to_string(), trace, line!(), file!().to_owned(), actions)
        }
    };
    ($msg:expr, $actions:expr) => {
        {
            let trace = backtrace::Backtrace::new();
            BrainFailure::new($msg.to_string(), trace, line!(), file!().to_owned(), $actions)
        }
    };
}
