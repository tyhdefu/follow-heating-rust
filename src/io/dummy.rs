use std::sync::mpsc;
use std::sync::mpsc::{Receiver, Sender, TryRecvError};

pub trait DummyIO {
    type MessageType;

    fn create() -> (Self, Sender<Self::MessageType>) where Self: Sized {
        let (sender, receiver) = mpsc::channel();
        let dummy_obj = Self::new(receiver);
        return (dummy_obj, sender);
    }

    fn new(receiver: Receiver<Self::MessageType>) -> Self;
}

pub fn read_all<T, F>(receiver: &Receiver<T>, on_value: F)
    where F: Fn(T) {
    loop {
        match receiver.try_recv() {
            Ok(x) => on_value(x),
            Err(TryRecvError::Empty) => break,
            Err(TryRecvError::Disconnected) => panic!("Disconnected!")
        }
    }
}