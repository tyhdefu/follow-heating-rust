use crate::io::temperatures::{Sensor, TemperatureManager};
use async_trait::async_trait;
use num_traits::cast::ToPrimitive;
use sqlx::types::BigDecimal;
use sqlx::Row;
use sqlx::{Executor, MySqlPool};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug)]
pub struct DBSensor {
    db_id: u32,
    purpose: String,
}

impl DBSensor {
    pub fn new(db_id: u32, purpose: String) -> DBSensor {
        DBSensor { db_id, purpose }
    }

    pub fn get_db_id(&self) -> u32 {
        self.db_id
    }

    pub fn get_purpose(&self) -> &str {
        &self.purpose
    }
}

#[derive(Debug)]
pub struct ThermisterCalibration {
    b_coefficient: f64,
    resistor: f64,
    raw_offset: i32,
}

const RESISTOR_IN_SERIES: f64 = 10000.0;
const TEMPERATURE_NOMINAL: f64 = 25.0;
const KELVIN_TO_CELCIUS: f64 = 273.15;

impl ThermisterCalibration {
    pub fn new(b_coefficient: f64, resistor: f64, raw_offset: i32) -> ThermisterCalibration {
        ThermisterCalibration {
            b_coefficient,
            resistor,
            raw_offset,
        }
    }

    pub fn apply(&self, raw_value: u32) -> f64 {
        let raw_value = raw_value as i32;
        let resistance: f64 = (1023f64 / ((raw_value + self.raw_offset) as f64)) - 1f64;
        let resistance = RESISTOR_IN_SERIES / resistance;
        let mut steinhart = resistance / self.resistor;
        steinhart = steinhart.ln();
        steinhart /= self.b_coefficient;
        steinhart += 1f64 / (TEMPERATURE_NOMINAL + KELVIN_TO_CELCIUS);
        steinhart = 1f64 / steinhart;
        return steinhart - KELVIN_TO_CELCIUS;
    }
}

pub async fn retrieve_temperatures(
    sensors: &Arc<Vec<(DBSensor, ThermisterCalibration)>>,
    pool: &MySqlPool,
) -> Result<HashMap<Sensor, f32>, String> {
    let mut temp_map = HashMap::new();

    //let start = Instant::now();

    // TODO: Check timestamp of data.
    let mut conn = pool
        .acquire()
        .await
        .map_err(|err| format!("Failed to acquire a connection from the pool {:?}", err))?;
    //let transaction = pool.begin()..await.expect("Expected to be able to begin transaction");
    for (sensor, calibration) in sensors.iter() {
        let row = sqlx::query!(
            "SELECT raw_value FROM reading WHERE sensor_id=? ORDER BY `id` DESC LIMIT 1",
            sensor.get_db_id()
        )
        .fetch_one(&mut conn)
        .await
        .map_err(|e| format!("Expected to find reading: {}", e))?;
        //.expect(&*("Failed to retrieve latest raw value for sensor ".to_owned() + sensor.get_purpose()));
        let raw_value: i32 = row.raw_value.unwrap() as i32;
        //println!("{} Raw value: {}. Calibration {:?}", sensor.get_purpose(), raw_value, calibration);
        let temp = calibration.apply(raw_value as u32);
        temp_map.insert(sensor.get_purpose().into(), temp as f32);
    }

    Ok(temp_map)
}

pub struct DBTemperatureManager {
    sensors_cache: Arc<Vec<(DBSensor, ThermisterCalibration)>>,
    conn: MySqlPool,
}

impl DBTemperatureManager {
    pub fn new(conn: MySqlPool) -> DBTemperatureManager {
        DBTemperatureManager {
            sensors_cache: Arc::new(Vec::new()),
            conn,
        }
    }
}

#[async_trait]
impl TemperatureManager for DBTemperatureManager {
    async fn retrieve_sensors(&mut self) -> Result<(), String> {
        let rows = self
            .conn
            .fetch_all(sqlx::query!("SELECT * FROM sensor WHERE type='MCP'"))
            .await
            .map_err(|e| format!("Expected to be able to retrieve MCP sensors {}", e))?;

        let mut new_sensors = Vec::new();
        for row in rows {
            let id: u32 = row.get("id");
            let purpose: String = row.get("purpose");
            let resistor: BigDecimal = row.get("calibration_1");
            let b_coefficient: BigDecimal = row.get("calibration_2");
            let raw_offset: BigDecimal = row.get("calibration_3");
            new_sensors.push((
                DBSensor::new(id, purpose),
                ThermisterCalibration::new(
                    b_coefficient.to_f64().unwrap(),
                    resistor.to_f64().unwrap(),
                    raw_offset.to_i32().unwrap(),
                ),
            ))
        }
        self.sensors_cache = Arc::new(new_sensors);
        Ok(())
    }

    async fn retrieve_temperatures(&self) -> Result<HashMap<Sensor, f32>, String> {
        retrieve_temperatures(&self.sensors_cache, &self.conn).await
    }
}

