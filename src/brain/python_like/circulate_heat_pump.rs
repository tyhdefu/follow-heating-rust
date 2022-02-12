use std::time::Instant;
use futures::FutureExt;
use tokio::sync::mpsc;
use tokio::sync::mpsc::Sender;
use tokio::task::JoinHandle;

#[derive(Debug)]
pub enum CirculateStatus {
    Uninitialised,
    Active(CirculateHeatPumpOnlyTaskHandle),
    Stopping(StoppingStatus)
}

#[derive(Debug)]
pub struct StoppingStatus {
    join_handle: Option<JoinHandle<()>>,
    sender: Sender<CirculateHeatPumpOnlyTaskMessage>,
    sent_terminate_request: Instant,
}

impl StoppingStatus {

    pub fn stopped() -> Self {
        let (tx, _) = mpsc::channel(1);
        Self {
            join_handle: None,
            sender: tx,
            sent_terminate_request: Instant::now(),
        }
    }

    pub fn sent_terminate_request_time(&self) -> &Instant {
        &self.sent_terminate_request
    }

    pub fn check_ready(&mut self) -> bool {
        match &mut self.join_handle {
            None => true,
            Some(handle) => {
                if handle.now_or_never().is_some() {
                    self.join_handle.take();
                    return true;
                }
                false
            },
        }
    }
}

#[derive(Debug)]
pub struct CirculateHeatPumpOnlyTaskHandle {
    join_handle: JoinHandle<()>,
    sender: Sender<CirculateHeatPumpOnlyTaskMessage>,
}

impl CirculateHeatPumpOnlyTaskHandle {

    pub fn new(join_handle: JoinHandle<()>, sender: Sender<CirculateHeatPumpOnlyTaskMessage>) -> Self {
        CirculateHeatPumpOnlyTaskHandle {
            join_handle,
            sender,
        }
    }

    pub fn join_handle(&mut self) -> &mut JoinHandle<()> {
        &mut self.join_handle
    }

    pub fn terminate_soon(self, leave_on: bool) -> StoppingStatus {
        self.sender.try_send(CirculateHeatPumpOnlyTaskMessage::new(leave_on))
            .expect("Should be able to send message");

        StoppingStatus {
            join_handle: Some(self.join_handle),
            sender: self.sender,
            sent_terminate_request: Instant::now(),
//            ready: false
        }
    }
}

#[derive(Debug)]
pub struct CirculateHeatPumpOnlyTaskMessage {
    leave_on: bool
}

impl CirculateHeatPumpOnlyTaskMessage {
    pub fn new(leave_on: bool) -> Self {
        CirculateHeatPumpOnlyTaskMessage {
            leave_on
        }
    }

    pub fn leave_on(&self) -> bool {
        self.leave_on
    }
}
