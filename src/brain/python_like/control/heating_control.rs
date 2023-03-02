use crate::brain::BrainFailure;

pub trait HeatPumpControl {
    fn try_set_heat_pump(&mut self, on: bool) -> Result<(), BrainFailure>;

    fn try_get_heat_pump(&self) -> Result<bool, BrainFailure>;
}

pub trait HeatCirculationPumpControl {
    fn try_set_heat_circulation_pump(&mut self, on: bool) -> Result<(), BrainFailure>;

    fn try_get_heat_circulation_pump(&self) -> Result<bool, BrainFailure>;
}

pub trait HeatingControl: HeatPumpControl + HeatCirculationPumpControl + Send + 'static {
    fn as_hp(&mut self) -> &mut dyn HeatPumpControl;

    fn as_cp(&mut self) -> &mut dyn HeatCirculationPumpControl;
}