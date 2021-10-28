use sqlx::MySqlPool;
use crate::io::temperatures::database::DBTemperatureManager;
use crate::io::temperatures::TemperatureManager;

mod io;
mod config;

const DB_USERNAME: &str = "pi";
const DB_PASSWORD: &str = "****";
const DB_PORT: u32 = 3306;
const DB_NAME: &str = "heating";

fn main() {
    let db_url = format!("mysql://{}:{}@localhost:{}/{}", DB_USERNAME, DB_PASSWORD, DB_PORT, DB_NAME);
    let pool = futures::executor::block_on(MySqlPool::connect(&db_url)).expect("to connect");
    let mut temps = DBTemperatureManager::new(pool);
    futures::executor::block_on(temps.retrieve_sensors());
    let cur_temps = futures::executor::block_on(temps.retrieve_temperatures()).expect("Failed to retrieve temperatures");
    println!("{:?}", cur_temps);
}
