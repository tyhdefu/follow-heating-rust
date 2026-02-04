use std::time::Duration;

use log::info;
use strum_macros::EnumIter;

use crate::brain::BrainFailure;

/// Which configuration of valves to use in order to generate the given outcome.
#[derive(PartialEq, Debug, Clone, EnumIter)]
pub enum HeatPumpMode {
    /// Tank only
    HotWaterOnly,

    /// Heat exchanger only
    /// Normally this is used to heat the house with the circulation pump on
    /// (Could theoretically be used to raise HP water before directing it to the tank so as to
    ///  avoid initially cooling it, but this would lose some heat to the air and reduce thermal
    ///  efficiency)
    HeatingOnly,

    /// Both tank and the heat exchanger (maybe 60% hot water)
    /// Normally this is used to increase heat both the house and DHW at once using a lower
    /// flow temperature and longer run time than doing one after the other. 
    /// (If the circulation pump isn't running this could be used to increase the flow through
    ///  the heat pump, improving the efficiency of its heat exchanger, while focusing heat
    ///  at the top of the tank, at the cost of likely reduced net thermal efficiency and heat
    ///  losses from piping. This would be more useful if TKVO were made variable)
    MostlyHotWater,

    /// Both tank and the heat exchanger, but without the extra pump the flow is reversed through
    /// the tank causing hot water to spill out of the top and increase the heat exchanger flow
    /// temperature.
    /// This only makes sense with the circulation pump on to boost the initial heat up times,
    /// ideally after the tank was efficiently heated using MostlyHotWater rather than
    /// HotWaterOnly and/or with cheaper electricity.
    BoostedHeating,

    /// Both tank and heat exchanger, but with the heat pump off, external circuit isolated and
    /// the flow through the tank reversed so that the hottest water is extracted.
    /// This only makes sense with the circulation pump on to prove low level heat rather than
    /// firing up the heat pump for a short period of time, or during a time of expensive
    /// electricity, ideally after the tank was efficiently heated using MostlyHotWater and/or
    /// with cheap electricity.
    DrainTank,

    /// Off state with nothing occurring
    /// However, the circulation pump could still be on, equalising the heating circuit
    /// temperature until it gets low enough for another heating mode to be required.
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
