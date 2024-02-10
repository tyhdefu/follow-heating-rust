use serde::Deserialize;
use serde_with::serde_as;
#[allow(unused_imports)]
use serde_with::DurationSeconds;
use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;
use std::time::Duration;

#[derive(Deserialize, Clone)]
pub struct Config {
    database: DatabaseConfig,
    wiser: WiserConfig,
    live_data: LiveDataConfig,
    devices: DevicesFromFileConfig,
    #[serde(default)]
    controls: ControlConfig,
}

impl Config {
    pub fn new(
        database: DatabaseConfig,
        wiser: WiserConfig,
        live_data: LiveDataConfig,
        devices: DevicesFromFileConfig,
        controls: ControlConfig,
    ) -> Self {
        Self {
            database,
            wiser,
            live_data,
            devices,
            controls,
        }
    }

    pub fn get_database(&self) -> &DatabaseConfig {
        &self.database
    }

    pub fn get_wiser(&self) -> &WiserConfig {
        &self.wiser
    }

    pub fn get_devices(&self) -> &DevicesFromFileConfig {
        &self.devices
    }

    pub fn get_live_data(&self) -> &LiveDataConfig {
        &self.live_data
    }

    pub fn get_control_config(&self) -> &ControlConfig {
        &self.controls
    }
}

#[derive(Deserialize, Clone)]
pub struct DatabaseConfig {
    user: String,
    password: String,
    port: u32,
    database: String,
}

impl DatabaseConfig {
    pub fn get_user(&self) -> &str {
        &self.user
    }

    pub fn get_password(&self) -> &str {
        &self.password
    }

    pub fn get_port(&self) -> u32 {
        self.port
    }

    pub fn get_database(&self) -> &str {
        &self.database
    }
}

#[derive(Deserialize, Clone)]
pub struct WiserConfig {
    ip: IpAddr,
    secret: String,
}

impl WiserConfig {
    pub fn fake() -> Self {
        WiserConfig {
            ip: Ipv4Addr::UNSPECIFIED.into(),
            secret: "".to_owned(),
        }
    }

    pub fn get_ip(&self) -> &IpAddr {
        &self.ip
    }

    pub fn get_secret(&self) -> &str {
        &self.secret
    }
}

#[derive(Deserialize, Clone)]
pub struct DevicesFromFileConfig {
    /// The file to read from to obtain the device activity data.
    file: String,
    /// The maximum number of minutes ago the device must have been detected in order to qualify
    /// it as being "active"
    active_within_minutes: usize,
}

impl DevicesFromFileConfig {
    pub fn get_file(&self) -> &str {
        &self.file
    }

    pub fn get_active_within_minutes(&self) -> usize {
        self.active_within_minutes
    }
}

#[derive(Deserialize, Clone)]
pub struct LiveDataConfig {
    wiser_file: PathBuf,
    temps_file: PathBuf,
}

impl LiveDataConfig {
    pub fn wiser_file(&self) -> &PathBuf {
        &self.wiser_file
    }

    pub fn temps_file(&self) -> &PathBuf {
        &self.temps_file
    }
}

#[serde_as]
#[derive(Deserialize, Clone)]
pub struct ControlConfig {
    /// How long to wait (in seconds) for a valve to
    /// start opening before waiting [valve_change_sleep] seconds
    /// The total time given for the valve to open [valve_start_open_sleep] + [valve_change_sleep]
    #[serde_as(as = "DurationSeconds")]
    valve_start_open_secs: Duration,
    /// The amount of time to wait for a valve to open / close, in addition
    /// to [valve_start_open]. This time is waited for all valves (opening and closing).
    #[serde_as(as = "DurationSeconds")]
    valve_change_secs: Duration,
    /// The amount of time to wait after a pump stops before playing with valves
    #[serde_as(as = "DurationSeconds")]
    pump_water_slow_secs: Duration,
    /// The extra amount of time to wait for water to slow compared to [pump_water_slow_secs]
    #[serde_as(as = "DurationSeconds")]
    extra_heat_pump_water_slow_secs: Duration,
}

impl Default for ControlConfig {
    fn default() -> Self {
        Self {
            valve_start_open_secs: Duration::from_secs(5),
            valve_change_secs: Duration::from_secs(3),
            pump_water_slow_secs: Duration::from_secs(2),
            extra_heat_pump_water_slow_secs: Duration::from_secs(3),
        }
    }
}

impl ControlConfig {
    pub fn get_valve_start_open_time(&self) -> &Duration {
        &self.valve_start_open_secs
    }

    pub fn get_valve_change_time(&self) -> &Duration {
        &self.valve_change_secs
    }

    pub fn get_pump_water_slow_time(&self) -> &Duration {
        &self.valve_change_secs
    }

    pub fn get_heat_pump_water_slow_time(&self) -> &Duration {
        &self.extra_heat_pump_water_slow_secs
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::net::Ipv4Addr;

    #[test]
    fn test_serialize() {
        let config_str = fs::read_to_string("test/testconfig.toml")
            .expect("Unable to read test config file. Is it missing?");
        let config: Config = toml::from_str(&config_str).expect("Error reading test config file");
        assert_eq!(config.database.user, "exampleuser");
        assert_eq!(config.database.password, "dbpassword");
        assert_eq!(config.database.port, 3306);
        assert_eq!(config.database.database, "heating");

        assert_eq!(config.wiser.ip, Ipv4Addr::new(192, 168, 0, 9));
        assert_eq!(config.wiser.secret, "super-secret-secret");

        let mut live_data_path = PathBuf::new();
        live_data_path.push("live_data");
        let mut temps_file = live_data_path.clone();
        temps_file.push("temps.json");
        let mut wiser_file = live_data_path.clone();
        wiser_file.push("wiser.json");

        assert_eq!(config.live_data.temps_file, temps_file);
        assert_eq!(config.live_data.wiser_file, wiser_file);

        assert_eq!(config.devices.file, "x.txt");
        assert_eq!(config.devices.active_within_minutes, 30);
    }
}
