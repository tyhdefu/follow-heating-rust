use std::time::Instant;
use tokio::sync::mpsc::Sender;
use tokio::task::JoinHandle;
use crate::brain::python_like::circulate_heat_pump::CirculateHeatPumpOnlyTaskHandle;

pub struct PumpPulseTaskHandle {
    join_handle: JoinHandle<()>,
    sender: Sender<CirculateHeatPumpOnlyTaskHandle>,
    sent_terminate_request: Option<Instant>,
}

