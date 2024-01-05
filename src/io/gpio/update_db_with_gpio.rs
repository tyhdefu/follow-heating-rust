use crate::io::gpio::{GPIOState, PinUpdate};
use log::{debug, error, info, warn};
use sqlx::{Executor, MySqlPool, Row};
use std::collections::HashMap;
use tokio::sync::mpsc::Receiver;

pub async fn run(conn: MySqlPool, mut receiver: Receiver<PinUpdate>) {
    info!("Running database GPIO updater.");
    let mut map: HashMap<u32, u32> = HashMap::new();

    let result = conn
        .fetch_all(sqlx::query!(
            "SELECT id, channel FROM sensor WHERE type='GPIO'"
        ))
        .await;

    if result.is_err() {
        error!(
            "Failed to fetch GPIO sensors from DB {:?} - WONT BE ABLE TO RECORD INTO DB",
            result.unwrap_err()
        );
        return;
    }

    let rows = result.unwrap();
    for row in rows {
        let channel: String = row.get("channel");
        map.insert(channel.parse().unwrap(), row.get("id"));
    }
    let map = map;
    debug!("Sensor Map: {:?}", map);

    loop {
        let result = receiver.recv().await;
        if result.is_none() {
            warn!("Sender seems to have been dropped for the database gpio updater.");
            break;
        }
        let pin_update = result.unwrap();

        let pin = pin_update.pin as u32;
        if let Some(sensor_id) = map.get(&pin) {
            let to = gpio_state_to_on_off(&pin_update.to);
            conn.execute(sqlx::query!(
                "INSERT INTO reading (sensor_id, raw_value) VALUES (?,?)",
                sensor_id,
                to
            ))
            .await
            .unwrap();
        } else {
            error!("No database entry found for gpio pin: {}", pin)
        }
    }

    fn gpio_state_to_on_off(state: &GPIOState) -> u16 {
        if let GPIOState::Low = state {
            1
        } else {
            0
        }
    }
}

