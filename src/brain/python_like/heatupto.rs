use std::fmt::{Debug, Display, Formatter};
use chrono::{DateTime, SecondsFormat, Utc};
use crate::python_like::heating_mode::TargetTemperature;
use crate::time::timeslot::ZonedSlot;

#[derive(Debug)]
pub struct HeatUpTo {
    target: TargetTemperature,
    expire: HeatUpEnd,
}

#[derive(Debug, PartialEq, Clone)]
pub enum HeatUpEnd {
    Slot(ZonedSlot),
    Utc(DateTime<Utc>),
}

impl Display for HeatUpEnd {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            HeatUpEnd::Slot(slot) => {
                write!(f, "During {}", slot)
            }
            HeatUpEnd::Utc(time) => {
                write!(f, "Until {}", time.to_rfc3339_opts(SecondsFormat::Millis, true))
            }
        }
    }
}

impl HeatUpTo {
    pub fn from_slot(target: TargetTemperature, expire: ZonedSlot) -> Self {
        Self {
            target,
            expire: HeatUpEnd::Slot(expire),
        }
    }

    pub fn from_time(target: TargetTemperature, expire: DateTime<Utc>) -> Self {
        Self {
            target,
            expire: HeatUpEnd::Utc(expire),
        }
    }

    pub fn has_expired(&self, now: DateTime<Utc>) -> bool {
        match &self.expire {
            HeatUpEnd::Slot(slot) => !slot.contains(&now),
            HeatUpEnd::Utc(expire_time) => now > *expire_time,
        }
    }

    pub fn get_target(&self) -> &TargetTemperature {
        &self.target
    }

    pub fn get_expiry(&self) -> &HeatUpEnd {
        &self.expire
    }
}