use serde::Deserialize;

#[derive(Deserialize)]
struct Config {
    database: DatabaseConfig
}

#[derive(Deserialize)]
struct DatabaseConfig {
    user: String,
    password: String,
    port: u32,
    database: String
}

mod tests {
    use std::fs;
    use crate::config::Config;

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
    }
}