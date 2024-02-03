use crate::brain::BrainFailure;

/// Which configuration of valves to use in order to generate the given outcome.
#[derive(PartialEq, Debug, Clone)]
pub enum HeatPumpMode {
    /// Heat only the tank.
    HotWaterOnly,
    /// Heat only the heating, skipping the tank
    HeatingOnly,
    /// Heat both at the same time (mostly does hot water)
    MostlyHotWater,
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
}

pub trait HeatCirculationPumpControl {
    fn try_set_heat_circulation_pump(&mut self, on: bool) -> Result<(), BrainFailure>;

    fn try_get_heat_circulation_pump(&self) -> Result<bool, BrainFailure>;
}

pub trait HeatingControl: HeatPumpControl + HeatCirculationPumpControl + Send + 'static {
    fn as_hp(&mut self) -> &mut dyn HeatPumpControl;

    fn as_cp(&mut self) -> &mut dyn HeatCirculationPumpControl;
}
