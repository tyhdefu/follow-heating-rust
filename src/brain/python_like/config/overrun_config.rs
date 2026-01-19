use crate::python_like::modes::heating_mode::PossibleTemperatureContainer;
use crate::time_util::timeslot::ZonedSlot;
use crate::Sensor;
use chrono::{DateTime, Utc};
use itertools::Itertools;
use log::{debug, error, info, trace};
use serde::Deserialize;
use std::collections::HashMap;
use std::fmt::{Display, Formatter};

#[derive(Deserialize, Clone, Debug, PartialEq, Default)]
pub struct OverrunConfig {
    pub slots: Vec<DhwBap>,
}

impl OverrunConfig {
    #[cfg(test)]
    pub fn new(slots: Vec<DhwBap>) -> Self {
        Self { slots }
    }

    pub fn combine(&mut self, mut other: OverrunConfig) {
        self.slots.append(&mut other.slots);
    }

    fn _get_current_slots<'a>(&'a self, now: &DateTime<Utc>) -> HashMap<Sensor, Vec<&'a DhwBap>> {
        trace!(
            "All slots: {}",
            self.slots.iter().map(|s| format!("{{ {} }}", s)).join(", ")
        );
        self
            .slots
            .iter()
            .filter(|slot| slot.slot.contains(now))
            .filter(|slot| {
                if slot.temps.max <= slot.temps.min {
                    error!("Invalid slot, slot max temp ({}) must be greater than the slot min temp ({}).", slot.temps.max, slot.temps.min);
                    return false;
                }
                if slot.temps.extra.is_some() && slot.temps.extra <= Some(slot.temps.max) {
                    error!("Invalid slot, slot extra temp ({:?}) must be greater than the slot max temp ({}).", slot.temps.extra, slot.temps.max);
                    return false;
                }
                return true;
            })
            .map(|slot| (slot.temps.sensor.clone(), slot))
            .into_group_map()
    }

    /// Get the best slot matching the specified function
    /// TKTP specifications trump other positions if the sensor it is under temperature
    /// Otherwise the sensor with the highest min, then max, then extra is used
    pub fn find_best_slot<T: PossibleTemperatureContainer>(&self,
        debug:   bool,
        now:     &DateTime<Utc>,
        temps:   &T,
        matches: impl Fn(&DhwTemps, f32) -> bool,
    ) -> Option<&DhwBap> {
        let applicable = self._get_current_slots(now);

        debug!("Current overrun time slots: {applicable:?}");

        let mut result: Option<&DhwBap> = None;

        for (sensor, baps) in &applicable {
            if let Some(temp) = temps.get_sensor_temp(sensor) {
                for bap in baps {
                    debug!(target: OVERRUN_LOG_TARGET, "Checking overrun for {}. Current temp {:.2}. Overrun config: {}", sensor, temp, bap);

                    if let Some(disable_below) = &bap.disable_below {
                        if let Some(temp) = temps.get_sensor_temp(&Sensor::TKEN) {
                            if *temp < disable_below.tken {
                                if debug {
                                    info!(target: OVERRUN_LOG_TARGET, "{bap}: * Overrun is disabled due to TKEN of {temp}");
                                }
                                continue;
                            }
                        }
                        else {
                            error!(target: OVERRUN_LOG_TARGET, "Potentially missing sensor: TKEN");
                        }

                        if let Some(temp) = temps.get_sensor_temp(&Sensor::TKBT) {
                            if *temp < disable_below.tkbt {
                                if debug {
                                    info!(target: OVERRUN_LOG_TARGET, "{bap}: * Overrun is disabled due to TKBT of {temp}");
                                }
                                continue;
                            }
                        }
                        else {
                            error!(target: OVERRUN_LOG_TARGET, "Potentially missing sensor: TKBT");
                        }
                    }
                        
                    if matches(&bap.temps, *temp) {
                        if let Some(old) = result {
                            if (matches!(sensor, Sensor::TKTP) && !matches!(old.temps.sensor, Sensor::TKTP) && *temp < bap.temps.min)
                               || bap.temps.min > old.temps.min
                               || (bap.temps.min == old.temps.min && bap.temps.max > old.temps.max)
                               || (bap.temps.min == old.temps.min && bap.temps.max == old.temps.max && bap.temps.extra > old.temps.extra)
                            {
                                if debug {
                                    info!(target: OVERRUN_LOG_TARGET, "{bap}: * This is a better match ({sensor}={temp:.2})");
                                }
                                result = Some(*bap);
                            }
                            else if debug {
                                info!(target: OVERRUN_LOG_TARGET, "{bap}: * Prior match was better ({sensor}={temp:.2})");
                            }
                        }
                        else {
                            if debug {
                                info!(target: OVERRUN_LOG_TARGET, "{bap}: * This was the first match ({sensor}={temp:.2})");
                            }
                            result = Some(*bap);
                        }
                    }
                    else if debug {
                        info!(target: OVERRUN_LOG_TARGET, "{bap}: * This did not match criteria ({sensor}={temp:.2})");
                    }
                }
            } else {
                error!(target: OVERRUN_LOG_TARGET, "Potentially missing sensor: {}", sensor);
            }
        }

        if !debug {
            info!(target: OVERRUN_LOG_TARGET, "Using {bap} ({sensor}={temp:.2})")
        }
        
        result
    }
}

pub const OVERRUN_LOG_TARGET: &str = "overrun";

/// A boost applicable at a certain time of day.
#[derive(Deserialize, PartialEq, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct DhwBap {
    /// The time slot during which this is applicable.
    pub slot: ZonedSlot,
    pub disable_below: Option<DisableBelow>,
    pub temps: DhwTemps,

    /// The maximum drop across HPFL / HPRT before the
    /// heat exchanger valve will be opened to increase
    /// the flow through the heat pump
    pub bypass: Option<Bypass>,

    pub mixed: Option<Mixed>,
}

#[derive(Deserialize, PartialEq, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct DhwTemps {
    /// The sensor to reach the temperature
    pub sensor: Sensor,

    /// The minimum allowed temperature during this slot.
    /// If the temperature is below this, then it will be heated as a priority.
    pub min: f32,

    /// The target target temperature when heating the water. If the heat pump is on
    /// for any reason it will stay on to reach this.
    pub max: f32,

    /// A higher maximum that will be used to increase efficiency:
    /// * If there is an opportunity for mixed mode
    /// * If the heat pump has been running for only a short time
    pub extra: Option<f32>,
}

#[derive(Deserialize, PartialEq, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct DisableBelow {
    pub tken: f32,
    pub tkbt: f32,
}

#[derive(Deserialize, PartialEq, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct Bypass {
    pub start_hp_drop: f32,
    pub stop_hp_drop:  f32,
}

#[derive(Deserialize, PartialEq, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct Mixed {
    pub start_hpfl_tktp_diff: f32,
    pub stop_hpfl_tktp_diff:  f32,
}

impl DhwBap {
    #[cfg(test)]
    pub fn _new(slot: ZonedSlot, sensor: Sensor, min_temp: f32, max_temp: f32) -> Self {
        assert!(min_temp < max_temp, "min_temp should be less than max_temp");
        Self {
            slot,
            disable_below: None,
            temps: DhwTemps {
                sensor, min: min_temp, max: max_temp, extra: None
            },
            bypass: None,
            mixed: None,
        }
    }
}

impl Display for DhwBap {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "DHW for {}: {:.1}-{:.1}/{:.1?} during {}",
            self.temps.sensor, self.temps.min, self.temps.max, self.temps.extra, self.slot
        )
    }
}

#[allow(clippy::zero_prefixed_literal)]
#[cfg(test)]
mod tests {
    use super::*;
    use crate::time_util::test_utils::{date, time};
    use chrono::{NaiveDateTime, TimeZone};

    #[test]
    fn test_deserialize() {
        let config_str = std::fs::read_to_string("test/python_brain/overrun_config/basic.toml")
            .expect("Failed to read config file.");
        let overrun_config: OverrunConfig =
            toml::from_str(&config_str).expect("Failed to deserialize config");

        let utc_slot = (time(03, 02, 05)..time(07, 03, 09)).into();
        let local_slot = (time(12, 45, 31)..time(14, 55, 01)).into();
        let local_slot2 = (time(09, 37, 31)..time(11, 15, 26)).into();

        let expected = vec![
            DhwBap::_new(ZonedSlot::Utc(utc_slot),      Sensor::TKTP, 0.0, 32.8),
            DhwBap::_new(ZonedSlot::Local(local_slot),  Sensor::TKBT, 0.0, 27.3),
            DhwBap::_new(ZonedSlot::Local(local_slot2), Sensor::TKTP, 30.0, 45.0),
        ];
        assert_eq!(overrun_config.slots, expected);
    }

    fn mk_map(bap: &DhwBap) -> HashMap<Sensor, Vec<&DhwBap>> {
        let mut map = HashMap::new();
        map.insert(bap.temps.sensor.clone(), vec![bap]);
        map
    }

    fn mk_map2<'a>(bap1: &'a DhwBap, bap2: &'a DhwBap) -> HashMap<Sensor, Vec<&'a DhwBap>> {
        let mut map = HashMap::new();
        if bap2.temps.sensor == bap1.temps.sensor {
            map.insert(bap1.temps.sensor.clone(), vec![bap1, bap2]);
        }
        else {
            map.insert(bap1.temps.sensor.clone(), vec![bap1]);
            map.insert(bap2.temps.sensor.clone(), vec![bap2]);
        }
        map
    }

    #[test]
    fn test_get_slot() {
        let utc_slot1 = (time(03, 02, 05)..time(07, 03, 09)).into();
        let utc_slot2 = (time(12, 45, 31)..time(14, 55, 01)).into();
        let utc_slot3 = (time(09, 37, 31)..time(11, 15, 26)).into();
        let utc_slot4 = (time(02, 00, 00)..time(04, 30, 00)).into();

        let slot1 = DhwBap::_new(ZonedSlot::Utc(utc_slot1), Sensor::TKTP, 0.0, 32.8);
        let slot2 = DhwBap::_new(ZonedSlot::Utc(utc_slot2), Sensor::TKTP, 0.0, 27.3);
        let slot3 = DhwBap::_new(ZonedSlot::Utc(utc_slot3), Sensor::TKTP, 30.0, 45.0);
        let slot4 = DhwBap::_new(ZonedSlot::Utc(utc_slot4), Sensor::TKTP, 25.0, 29.5);

        let config = OverrunConfig::new(vec![
            slot1.clone(),
            slot2.clone(),
            slot3.clone(),
            slot4.clone(),
        ]);

        let irrelevant_day = date(2022, 04, 18);
        let time1 =
            Utc::from_utc_datetime(&Utc, &NaiveDateTime::new(irrelevant_day, time(06, 23, 00)));

        assert_eq!(
            config._get_current_slots(&time1),
            mk_map(&slot1),
            "Simple"
        );

        let slot_1_and_4_time =
            Utc::from_utc_datetime(&Utc, &NaiveDateTime::new(irrelevant_day, time(03, 32, 00)));
        //assert_eq!(config.get_current_slots(slot_1_and_4_time, true).get_applicable(), &mk_map(&slot1), "Slot 1 because its hotter than Slot 4"); // No longer applicable because it returns both, not the best one.
        assert_eq!(
            config._get_current_slots(&slot_1_and_4_time),
            mk_map2(&slot1, &slot4),
            "Both"
        );
    }

    #[test]
    fn test_overlapping_min_temp() {
        let datetime = Utc::from_utc_datetime(
            &Utc,
            &NaiveDateTime::new(date(2022, 08, 19), time(04, 15, 00)),
        );

        let utc_slot1 = (time(04, 00, 00)..time(04, 30, 00)).into();
        let utc_slot2 = (time(03, 00, 00)..time(04, 30, 00)).into();
        let slot1 = DhwBap::_new(ZonedSlot::Utc(utc_slot1), Sensor::TKTP, 40.5, 43.6);
        let slot2 = DhwBap::_new(ZonedSlot::Utc(utc_slot2), Sensor::TKTP, 36.0, 41.6);

        let config = OverrunConfig::new(vec![slot1.clone(), slot2.clone()]);

        let mut temps = HashMap::new();
        temps.insert(Sensor::TKTP, 38.0); // A temp below the higher min temp.
        let slot = config.find_best_slot(false, &datetime, &temps,
            |temps, temp| (false || temp <= temps.min) && temp < temps.max
        );
        assert_eq!(slot, Some(&slot1));
    }

    #[test]
    fn test_disjoint_annoying() {
        let datetime = Utc::from_utc_datetime(
            &Utc,
            &NaiveDateTime::new(date(2022, 08, 19), time(04, 15, 00)),
        );

        let utc_slot1 = (time(04, 00, 00)..time(04, 30, 00)).into();
        let utc_slot2 = (time(03, 00, 00)..time(04, 30, 00)).into();

        let slot1 = DhwBap::_new(ZonedSlot::Utc(utc_slot1), Sensor::TKBT, 33.0, 35.0);
        let slot2 = DhwBap::_new(ZonedSlot::Utc(utc_slot2), Sensor::TKBT, 37.0, 43.0);

        let config = OverrunConfig::new(vec![slot1.clone(), slot2.clone()]);



        let current_tkbt_temp = 36.0; // Example of tkbt temp that should cause it to turn on due to slot2.
        let mut temps = HashMap::new();
        temps.insert(Sensor::TKBT, current_tkbt_temp);

        let slot = config.find_best_slot(false, &datetime, &temps,
            |temps, temp| (false || temp <= temps.min) && temp < temps.max
        );

        let bap = slot.expect("Should have a bap.");

        assert_eq!(bap, &slot2);
    }
}

