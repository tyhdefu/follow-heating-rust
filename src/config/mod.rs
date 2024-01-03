use serde::Deserialize;
use std::{
    net::{IpAddr, Ipv4Addr},
    path::PathBuf,
};

#[derive(Deserialize, Clone)]
pub struct Config {
    database: DatabaseConfig,
    wiser: WiserConfig,
    live_data: LiveDataConfig,
    devices: DevicesFromFileConfig,
}

impl Config {
    pub fn new(
        database: DatabaseConfig,
        wiser: WiserConfig,
        live_data: LiveDataConfig,
        devices: DevicesFromFileConfig,
    ) -> Self {
        Self {
            database,
            wiser,
            live_data,
            devices,
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
