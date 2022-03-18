use std::collections::HashMap;
use sqlx::{MySqlPool, Executor, Row};
use tokio::sync::mpsc::Receiver;
use crate::io::gpio::{GPIOState, PinUpdate};

pub async fn run(conn: MySqlPool, mut receiver: Receiver<PinUpdate>) {

    println!("Running database GPIO updater.");
    let mut map: HashMap<u32, u32> = HashMap::new();

    let result = conn.fetch_all(sqlx::query!("SELECT id, channel FROM sensor WHERE type='GPIO'"))
        .await;

    if result.is_err() {
        eprintln!("Failed to fetch GPIO sensors from DB {:?} - WONT BE ABLE TO RECORD INTO DB", result.unwrap_err());
        return;
    }

    let rows = result.unwrap();
    for row in rows {
        let channel: String = row.get("channel");
        map.insert(channel.parse().unwrap(), row.get("id"));
    }
    let map = map;
    println!("Sensor Map: {:?}", map);

    loop {
        let result = receiver.recv().await;
        if result.is_none() {
            println!("Sender seems to have been dropped for the database gpio updater.");
            break;
        }
        let pin_update = result.unwrap();

        let pin = pin_update.pin as u32;
        if let Some(sensor_id) = map.get(&pin) {
            let to = gpio_state_to_on_off(&pin_update.to);
            conn.execute(sqlx::query!("INSERT INTO reading (sensor_id, raw_value) VALUES (?,?)", sensor_id, to)).await.unwrap();
        }
        else {
            eprintln!("No database entry found for gpio pin: {}", pin)
        }
    }

    fn gpio_state_to_on_off(state: &GPIOState) -> u16 {
        if let GPIOState::LOW = state {
            1
        }
        else {
            0
        }
    }
}