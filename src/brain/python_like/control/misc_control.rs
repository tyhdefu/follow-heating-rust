use crate::brain::BrainFailure;

pub trait MiscControls: ImmersionHeaterControl + WiserPowerControl {}

pub trait ImmersionHeaterControl {
    fn try_set_immersion_heater(&mut self, on: bool) -> Result<(), BrainFailure>;

    fn try_get_immersion_heater(&self) -> Result<bool, BrainFailure>;
}

pub trait WiserPowerControl {
    fn try_set_wiser_power(&mut self, on: bool) -> Result<(), BrainFailure>;

    fn try_get_wiser_power(&mut self) -> Result<bool, BrainFailure>;
}