use crate::brain::boost_active_rooms::update_boosted_rooms;
use crate::brain::boost_active_rooms::AppliedBoosts;
use crate::brain::immersion_heater::follow_ih_model;
use crate::brain::modes::heating_mode::{HeatingMode, SharedData};
use crate::brain::modes::intention::Intention;
use crate::brain::modes::working_temp::WorkingRange;
use crate::brain::modes::{HeatingState, InfoCache};
use crate::brain::python_like::control::devices::Device;
use crate::brain::{modes, Brain, BrainFailure};
use crate::io::IOBundle;
use crate::time_util::mytime::TimeProvider;
use config::PythonBrainConfig;
use itertools::Itertools;
use log::{debug, error, info, trace, warn};
use std::collections::HashSet;
use std::time::{Duration, Instant};
use tokio::runtime::Runtime;

use super::modes::working_temp::WorkingTemperatureRange;

pub mod config;
pub mod control;

#[cfg(test)]
mod test;

// Functions for getting the max working temperature.

pub struct FallbackWorkingRange {
    previous: Option<(WorkingRange, Instant)>,
    default:  WorkingRange,
}

impl FallbackWorkingRange {
    pub fn new(default: WorkingTemperatureRange) -> Self {
        FallbackWorkingRange {
            previous: None,
            default: WorkingRange::new(default, None),
        }
    }

    pub fn get_fallback(&self, timeout: Duration) -> &WorkingRange {
        if let Some((range, updated)) = &self.previous {
            if (*updated + timeout) > Instant::now() {
                warn!("Using last working range as fallback: {}", range);
                return range;
            }
        }
        warn!(
            "No recent previous range to use, using default {}",
            &self.default
        );
        &self.default
    }

    pub fn update(&mut self, range: &WorkingRange) {
        self.previous.replace((range.clone(), Instant::now()));
    }
}

pub struct PythonBrain {
    config: PythonBrainConfig,
    /// The current state. None if just started and need to figure out what state to be in.
    heating_mode: Option<HeatingMode>,
    shared_data: SharedData,
    applied_boosts: AppliedBoosts,
    /// Whether we just reloaded / just restarted
    /// This is used to print additional one-time debugging information.
    just_reloaded: bool,
}

impl PythonBrain {
    pub fn new(config: PythonBrainConfig) -> Self {
        Self {
            shared_data: SharedData::new(FallbackWorkingRange::new(
                config.default_working_range.clone(),
            )),
            config,
            heating_mode: None,
            applied_boosts: AppliedBoosts::new(),
            just_reloaded: true,
        }
    }

    fn provide_debug_info(
        &mut self,
        io_bundle: &mut IOBundle,
        time_provider: &impl TimeProvider,
    ) -> Result<(), BrainFailure> {
        // Provide information on what active devices have actually been seen.
        const CHECK_MINUTES: usize = 30;
        let active_devices: HashSet<Device> = io_bundle
            .active_devices()
            .get_active_devices_within(&time_provider.get_utc_time(), CHECK_MINUTES)?
            .into_iter()
            .collect();

        // Accumulate all devices and log which ones are found and which aren't
        let mut devices_in_config = HashSet::new();
        for part in self.config.get_boost_active_rooms().get_parts() {
            devices_in_config.insert(part.get_device().clone());
        }
        info!(
            "All devices used in config: {:?}",
            prettify_devices(devices_in_config.clone())
        );

        let mut found = HashSet::new();
        let mut not_found = HashSet::new();
        for device in devices_in_config.iter().cloned() {
            if active_devices.contains(&device) {
                found.insert(device);
            } else {
                not_found.insert(device);
            }
        }
        info!(
            "The following devices were found within the last {} minutes: {:?}",
            CHECK_MINUTES,
            prettify_devices(found)
        );
        info!(
            "The following devices were NOT found within the last {} minutes: {:?}",
            CHECK_MINUTES,
            prettify_devices(not_found)
        );

        let unused_devices =
            prettify_devices(active_devices.difference(&devices_in_config).cloned());
        info!(
            "The following devices were active but not used in the config: {:?}",
            unused_devices
        );

        Ok(())
    }
}

fn prettify_devices(list: impl IntoIterator<Item = Device>) -> Vec<String> {
    list.into_iter()
        .sorted()
        .map(|device| format!("{}", device))
        .collect_vec()
}

impl Default for PythonBrain {
    fn default() -> Self {
        PythonBrain::new(PythonBrainConfig::default())
    }
}

impl Brain for PythonBrain {
    fn run(
        &mut self,
        runtime: &Runtime,
        io_bundle: &mut IOBundle,
        time_provider: &impl TimeProvider,
    ) -> Result<(), BrainFailure> {
        if self.just_reloaded {
            self.provide_debug_info(io_bundle, time_provider)?;
            self.just_reloaded = false;
        }

        // Update our value of wiser's state if possible.
        match runtime
            .block_on(io_bundle.wiser().get_heating_on())
            .map(HeatingState::new)
        {
            Ok(wiser_heating_on_new) => {
                self.shared_data.last_successful_contact = Instant::now();
                if self.shared_data.last_wiser_state != wiser_heating_on_new {
                    self.shared_data.last_wiser_state = wiser_heating_on_new;
                    info!(target: "wiser", "Wiser heating state changed to {}", wiser_heating_on_new);
                }
            }
            Err(_) => {
                // The wiser hub often doesn't respond. If this happens, carry on heating for a maximum of 1 hour.
                error!(target: "wiser", "Failed to get whether heating was on. Using old value");
                if Instant::now() - self.shared_data.last_successful_contact
                    > Duration::from_secs(60 * 60)
                {
                    error!(target: "wiser", "Saying off - last successful contact too long ago: {}s ago", self.shared_data.last_successful_contact.elapsed().as_secs());
                    self.shared_data.last_wiser_state = HeatingState::OFF;
                }
            }
        }

        let working_temp_range = modes::heating_mode::get_working_temp_fn(
            self.shared_data.get_fallback_working_range(),
            io_bundle.wiser(),
            &self.config,
            runtime,
        );
        let mut wiser_heating_state = self.shared_data.last_wiser_state;

        let ignore_wiser_heating_slot = self
            .config
            .get_no_heating()
            .iter()
            .find(|slot| slot.contains(&time_provider.get_utc_time()));

        if let Some(slot) = ignore_wiser_heating_slot {
            debug!("Ignoring wiser heating due to slot: {slot}. Pretending its off. It was actually: {wiser_heating_state}");
            wiser_heating_state = HeatingState::OFF;
        }

        let mut info_cache = InfoCache::create(wiser_heating_state, working_temp_range);

        // Heating mode switches
        match &mut self.heating_mode {
            None => {
                warn!("No current mode - probably just started up - Running same logic as ending a state.");
                let intention = Intention::finish();
                let new_state = modes::heating_mode::handle_intention(
                    intention,
                    &mut info_cache,
                    io_bundle,
                    &self.config,
                    runtime,
                    &time_provider.get_utc_time(),
                )?;
                let mut new_mode = match new_state {
                    None => {
                        error!("Got no next state - should have had something since we didn't keep state. Going to off.");
                        HeatingMode::off()
                    }
                    Some(mode) => mode,
                };
                info!("Entering mode: {:?}", new_mode);
                new_mode.transition_to(&None, &self.config, runtime, io_bundle)?;
                self.heating_mode = Some(new_mode);
                self.shared_data.notify_entered_state();
            }
            Some(cur_mode) => {
                trace!("Current mode: {:?}", cur_mode);
                let next_mode = cur_mode.update(
                    &mut self.shared_data,
                    runtime,
                    &self.config,
                    io_bundle,
                    &mut info_cache,
                    time_provider,
                )?;
                if let Some(next_mode) = next_mode {
                    if &next_mode != cur_mode {
                        info!("Transitioning from {:?} to {:?}", cur_mode, next_mode);
                        let old_mode = std::mem::replace(cur_mode, next_mode);
                        cur_mode.transition_to(&Some(old_mode), &self.config, runtime, io_bundle)?;
                        self.shared_data.notify_entered_state();
                    } else {
                        info!("Next mode same as current. Not switching."); // TODO: Debug
                    }
                }
            }
        }

        // Immersion heater
        let temps = runtime.block_on(info_cache.get_temps(io_bundle.temperature_manager()));
        if temps.is_err() {
            error!(
                "Error retrieving temperatures: {}",
                temps.as_ref().unwrap_err()
            );
            if io_bundle.misc_controls().try_get_immersion_heater()? {
                error!("Turning off immersion heater since we didn't get temperatures");
                io_bundle.misc_controls().try_set_immersion_heater(false)?;
            }
            return Ok(());
        }
        let temps = temps.ok().unwrap();
        follow_ih_model(
            time_provider,
            &temps,
            io_bundle.misc_controls().as_ih(),
            self.config.get_immersion_heater_model(),
        )?;

        // Active device/room boosting.
        match io_bundle
            .active_devices()
            .get_active_devices(&time_provider.get_utc_time())
        {
            Ok(devices) => {
                match runtime.block_on(update_boosted_rooms(
                    &mut self.applied_boosts,
                    self.config.get_boost_active_rooms(),
                    devices,
                    io_bundle.wiser(),
                )) {
                    Ok(_) => {}
                    Err(error) => {
                        warn!("Error boosting active rooms: {}", error);
                    }
                }
            }
            Err(err) => error!("Error getting active devices: {}", err),
        }

        Ok(())
    }

    fn reload_config(&mut self) {
        match config::try_read_python_brain_config() {
            None => error!("Failed to read python brain config, keeping previous config"),
            Some(config) => {
                self.config = config;
                self.just_reloaded = true;
                info!("Reloaded config");
            }
        }
    }
}
