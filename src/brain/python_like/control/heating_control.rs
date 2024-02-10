use log::debug;

use crate::brain::BrainFailure;

/// Which configuration of valves to use in order to generate the given outcome.
#[derive(PartialEq, Debug, Clone)]
pub enum HeatPumpMode {
    /// Heat only the tank.
    HotWaterOnly,
    /// Heat only the heating, skipping the tank
    HeatingOnly,
    /// Heat both at the same time (mabe 60% hot water)
    MostlyHotWater,
    /// Heating, with some hot water spilling out of the top
    /// to potentially boost the heat pump
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
            HeatPumpMode::HotWaterOnly   => true,
            HeatPumpMode::HeatingOnly    => true,
            HeatPumpMode::MostlyHotWater => true,
            HeatPumpMode::BoostedHeating => true,
            HeatPumpMode::DrainTank      => false,
            HeatPumpMode::Off            => false,
        }
    }

    pub fn is_hp_off(&self) -> bool {
        !self.is_hp_on()
    }
}

pub trait HeatPumpControl {
    fn try_set_heat_pump(&mut self, mode: HeatPumpMode) -> Result<(), BrainFailure>;

    fn try_get_heat_pump(&self) -> Result<HeatPumpMode, BrainFailure>;

    fn set_heat_pump(&mut self, mode: HeatPumpMode, debug_message: Option<&'static str>) -> Result<(), BrainFailure> {
        if self.try_get_heat_pump()? != mode {
            if let Some(debug_message) = debug_message {
                debug!("{debug_message}");
            }
            self.try_set_heat_pump(mode)?;
        }
        Ok(())
    }
}

pub trait HeatCirculationPumpControl {
    fn try_set_heat_circulation_pump(&mut self, on: bool) -> Result<(), BrainFailure>;

    fn try_get_heat_circulation_pump(&self) -> Result<bool, BrainFailure>;

    fn set_heat_circulation_pump(&mut self, on: bool, debug_message: Option<&'static str>) -> Result<(), BrainFailure> {
        if self.try_get_heat_circulation_pump()? != on {
            if let Some(debug_message) = debug_message {
                debug!("{debug_message}");
            }
            self.try_set_heat_circulation_pump(on)?;
        }
        Ok(())
    }
}

pub trait HeatingControl: HeatPumpControl + HeatCirculationPumpControl + Send + 'static {
    fn as_hp(&mut self) -> &mut dyn HeatPumpControl;

    fn as_cp(&mut self) -> &mut dyn HeatCirculationPumpControl;
}
