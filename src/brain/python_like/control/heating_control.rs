use std::time::Duration;

use log::info;
use strum_macros::EnumIter;

use crate::brain::BrainFailure;

/// Which configuration of valves to use in order to generate the given outcome.
#[derive(PartialEq, Debug, Clone, EnumIter)]
pub enum HeatPumpMode {
    /// Heat only the tank.
    HotWaterOnly,
    /// As HotWaterOnly, except that the heat exchanger valve is open
    /// This should increase the flow through the heat pump and so improve efficiency,
    /// at the cost of losing some heat to the air and lower flow through the tank
    HeatingOnly,
    /// Heat both at the same time (mabe 60% hot water)
    MostlyHotWater,
    /// Heating, with some hot water spilling out of the top to potentially boost the heat pump
    /// OR HotWaterOnly, except that the heat exchanger valve is open to increase the flow through
    /// the heat pump and so improve efficiency, at the cost of losing some heat to the air and
    /// lower flow through the tank
    BoostedHeating,
    /// Heat pump off (and is blocked), secondary pump extracting heat out of the tank in order to cool the
    /// temperature.
    DrainTank,
    /// Neutral off state with nothing occurring.
    Off,
}

impl HeatPumpMode {
    pub fn is_hp_on(&self) -> bool {
        match self {
            HeatPumpMode::HotWaterOnly           => true,
            HeatPumpMode::HeatingOnly            => true,
            HeatPumpMode::MostlyHotWater         => true,
            HeatPumpMode::BoostedHeating         => true,
            HeatPumpMode::DrainTank              => false,
            HeatPumpMode::Off                    => false,
        }
    }

    pub fn is_hp_off(&self) -> bool {
        !self.is_hp_on()
    }
}

pub trait HeatPumpControl {
    fn try_set_heat_pump(&mut self, mode: HeatPumpMode) -> Result<(), BrainFailure>;

    fn try_get_heat_pump(&self) -> Result<HeatPumpMode, BrainFailure>;

    fn set_heat_pump(&mut self, new_mode: HeatPumpMode, message: Option<&'static str>) -> Result<(), BrainFailure> {
        let old_mode = self.try_get_heat_pump()?;
        if new_mode != old_mode {
            let duration = self.get_heat_pump_on_with_time()?.1;
            if let Some(message) = message {
                // TODO: "after" is since the last change to the heat pump on/off state, so the message
                // is a bit misleading given there are other states
                info!("{message} after {}", as_opt_hours_mins_secs(duration));
            }
            else {
                info!("Switched from {old_mode:?} to {new_mode:?} after {}", as_opt_hours_mins_secs(duration));
            }
            self.try_set_heat_pump(new_mode)?;
        }
        Ok(())
    }

    fn get_heat_pump_on_with_time(&self) -> Result<(bool, Duration), BrainFailure>;
}

pub trait HeatCirculationPumpControl {
    fn try_set_circulation_pump(&mut self, on: bool) -> Result<(), BrainFailure>;

    fn get_circulation_pump(&self) -> Result<(bool, Duration), BrainFailure>;

    fn set_circulation_pump(&mut self, new: bool, message: Option<&'static str>) -> Result<(), BrainFailure> {
        let (old, duration) = self.get_circulation_pump()?;
        if new != old {
            if let Some(message) = message {
                info!("{message} after {}", as_opt_hours_mins_secs(duration));
            }
            else {
                info!("Switched from {old:?} to {new:?} after {}", as_opt_hours_mins_secs(duration));
            }
            self.try_set_circulation_pump(new)?;
        }
        Ok(())
    }
}

pub trait HeatingControl: HeatPumpControl + HeatCirculationPumpControl + Send + 'static {
    fn as_hp(&mut self) -> &mut dyn HeatPumpControl;

    fn as_cp(&mut self) -> &mut dyn HeatCirculationPumpControl;
}

fn as_opt_hours_mins_secs(duration: Duration) -> String {
    let secs = duration.as_secs();
    let mins = secs/60;
    if mins >= 60 {
        return format!("{}h{}m{}s", mins/60, mins%60, secs%60);
    }
    else {
        return format!("{}m{}s", mins, secs%60);
    }
}
