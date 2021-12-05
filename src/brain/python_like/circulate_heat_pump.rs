use std::time::Instant;
use tokio::sync::mpsc::Sender;
use tokio::task::JoinHandle;

pub struct CirculateHeatPumpOnlyTaskHandle {
    join_handle: JoinHandle<()>,
    sender: Sender<CirculateHeatPumpOnlyTaskMessage>,
    sent_terminate_request: Option<Instant>,
}

impl CirculateHeatPumpOnlyTaskHandle {

    pub fn new(join_handle: JoinHandle<()>, sender: Sender<CirculateHeatPumpOnlyTaskMessage>) -> Self {
        CirculateHeatPumpOnlyTaskHandle {
            join_handle,
            sender,
            sent_terminate_request: None,
        }
    }

    pub fn join_handle(&mut self) -> &mut JoinHandle<()> {
        &mut self.join_handle
    }

    pub fn terminate_soon(&mut self, leave_on: bool) {
        if self.sent_terminate_request.is_none() {
            self.sender.try_send(CirculateHeatPumpOnlyTaskMessage::new(leave_on))
                .expect("Should be able to send message");
            self.sent_terminate_request = Some(Instant::now())
        }
    }

    pub fn get_sent_terminate_request(&self) -> &Option<Instant> {
        &self.sent_terminate_request
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
