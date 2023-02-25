use std::ops::{Deref, DerefMut};
use std::sync::{Arc, Mutex};
use crate::io::robbable::Dispatchable::Available;

pub enum Dispatchable<T> {
    Available(DispatchAvailable<T>),
    InUse(Robbable<T>),
    Changing
}

impl<T> Dispatchable<T> {
    pub fn of(value: T) -> Self {
        Available(DispatchAvailable::new(value))
    }

    pub fn rob_or_get_now(&mut self) -> Result<&mut DispatchAvailable<T>, ()> {
        match self {
            Dispatchable::Available(value) => Ok(value),
            Dispatchable::InUse(access) => {
                let taken = access.rob();
                if let Some(value) = taken {
                    *self = Dispatchable::of(value);
                    if let Dispatchable::Available(value) = self {
                        Ok(value)
                    }
                    else {
                        Err(())
                    }
                }
                else {
                    return Err(());
                }
            },
            Dispatchable::Changing => panic!("Dispatchable is still changing!"),
        }
    }
}

pub struct DispatchAvailable<T> {
    resource: T
}

impl<T> DispatchAvailable<T> {
    pub fn new(resource: T) -> Self {
        DispatchAvailable {
            resource
        }
    }

    pub fn dispatch(self) -> (Robbable<T>, DispatchedRobbable<T>){
        Robbable::create(self.resource)
    }
}

impl<T> Deref for DispatchAvailable<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.resource
    }
}

impl<T> DerefMut for DispatchAvailable<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.resource
    }
}

pub struct Robbable<T> {
    mutex: Arc<Mutex<Option<T>>>,
}

impl<T> Robbable<T> {
    pub fn create(resource: T) -> (Self, DispatchedRobbable<T>) {
        let mutex = Arc::new(Mutex::new(Some(resource)));
        let robbable = Robbable {
            mutex: mutex.clone(),
        };
        let dispatched = DispatchedRobbable::of(mutex);
        return (robbable, dispatched);
    }

    fn rob(&mut self) -> Option<T> {
        self.mutex.lock().unwrap().take()
    }
}

pub struct DispatchedRobbable<T> {
    resource: Arc<Mutex<Option<T>>>,
}

impl<T> DispatchedRobbable<T> {
    pub fn of(resource: Arc<Mutex<Option<T>>>) -> Self {
        DispatchedRobbable {
            resource,
        }
    }

    pub fn access(&self) -> &Mutex<Option<T>> {
        &self.resource
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct ImportantData {
        thing: i32
    }

    #[test]
    pub fn test_dispatch() {
        const THING_VALUE: i32 = 10;
        let data = ImportantData {
            thing: THING_VALUE
        };
        let dispatchable = Dispatchable::of(data);
        if let Dispatchable::Available(data) = dispatchable {
            let (mut robbable, dispatched) = data.dispatch();
            {
                // We can access this data from a seperate thread, for example.
                let guard = &dispatched.access().lock().unwrap();
                let taken_data = guard.as_ref().expect("Data should still be present.");
                assert_eq!(taken_data.thing, THING_VALUE, "Data should not have mutated.");
            }

            {
                // We can revoke access and stop the other thread accessing as soon as it releases its guard.
                let robbed = robbable.rob().expect("Should have succeeded in robbing the data");
                assert_eq!(robbed.thing, THING_VALUE, "Data should not have mutated after robbing.")
            }

            {
                // We no longer have access from the other thread
                let guard = &dispatched.access().lock().unwrap();
                assert!(guard.is_none(), "Data access should have been revoked.");
            }
        }
        else {
            panic!("Dispatchable::of did not give an available dispatchable");
        }
    }
}