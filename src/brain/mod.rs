use backtrace::Backtrace;
use tokio::runtime::Runtime;
use crate::io::gpio::GPIOManager;
use crate::io::IOBundle;
use crate::io::temperatures::TemperatureManager;
use crate::io::wiser::WiserManager;

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
    unknown_gpio_state: bool,
}

pub trait Brain {
    fn run<T, G, W>(&mut self, runtime: &Runtime, io_bundle: &mut IOBundle<T,G,W>) -> Result<(), BrainFailure>
        where
            T: TemperatureManager,
            W: WiserManager,
            G: GPIOManager + Send + 'static;
}

impl CorrectiveActions {

    pub fn new() -> Self {
        CorrectiveActions {
            unknown_gpio_state: false,
        }
    }

    pub fn unknown_gpio() -> Self {
        CorrectiveActions::new().with_gpio_unknown_state()
    }

    pub fn is_gpio_in_unknown_state(&self) -> bool {
        self.unknown_gpio_state
    }

    pub fn with_gpio_unknown_state(mut self) -> Self {
        self.unknown_gpio_state = true;
        self
    }
}
