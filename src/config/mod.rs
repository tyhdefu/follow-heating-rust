use std::net::{IpAddr, Ipv4Addr};
use serde::Deserialize;

use crate::io::devices::DevicesFromFileConfig;

#[derive(Deserialize, Clone)]
pub struct Config {
    database: DatabaseConfig,
    wiser: WiserConfig,
    devices: DevicesFromFileConfig,
}

impl Config {
    pub fn get_database(&self) -> &DatabaseConfig {
        &self.database
    }

    pub fn get_wiser(&self) -> &WiserConfig {
        &self.wiser
    }

    pub fn get_devices(&self) -> &DevicesFromFileConfig {
        &self.devices
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
    pub fn new(ip: IpAddr, secret: String) -> Self {
        WiserConfig {
            ip,
            secret
        }
    }

    pub fn fake() -> Self {
        WiserConfig {
            ip: Ipv4Addr::new(0, 0, 0, 0).into(),
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

#[cfg(test)]
mod tests {
    use std::fs;
    use std::net::Ipv4Addr;
    use super::*;

    #[test]
    fn test_serialize() {
        let config_str = fs::read_to_string("test/testconfig.toml")
            .expect("Unable to read test config file. Is it missing?");
        let config: Config = toml::from_str(&*config_str)
            .expect("Error reading test config file");
        assert_eq!(config.database.user, "exampleuser");
        assert_eq!(config.database.password, "dbpassword");
        assert_eq!(config.database.port, 3306);
        assert_eq!(config.database.database, "heating");

        assert_eq!(config.wiser.ip, Ipv4Addr::new(192, 168, 0, 9));
        assert_eq!(config.wiser.secret, "super-secret-secret");
    }
}