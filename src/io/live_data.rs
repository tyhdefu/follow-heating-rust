use std::{
    fmt::{self, Display},
    sync::Mutex,
};

use chrono::{DateTime, Utc};
use log::warn;

pub struct CheckAgeResult {
    max_age_seconds: i64,
    actual_age_seconds: i64,
    age_type: AgeType,
}

impl CheckAgeResult {
    pub fn age_type(&self) -> &AgeType {
        &self.age_type
    }
}

impl Display for CheckAgeResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:?}: {}s old, (max {}s)",
            self.age_type, self.actual_age_seconds, self.max_age_seconds
        )
    }
}

#[derive(Debug)]
pub enum AgeType {
    Good,
    GettingOld,
    TooOld,
}

pub fn check_age(timestamp: DateTime<Utc>, max_age: i64) -> CheckAgeResult {
    let age_seconds = Utc::now().signed_duration_since(timestamp).num_seconds();

    let age_type = if age_seconds > max_age {
        AgeType::TooOld
    } else if age_seconds > warn_age(max_age) {
        AgeType::GettingOld
    } else {
        AgeType::Good
    };

    CheckAgeResult {
        max_age_seconds: max_age,
        actual_age_seconds: age_seconds,
        age_type,
    }
}

// Warn at 3/4 of error age.
fn warn_age(error_age: i64) -> i64 {
    (error_age / 4) * 3
}

pub struct CachedPrevious<T: Clone> {
    data: Mutex<Option<T>>,
}

impl<T: Clone> CachedPrevious<T> {
    pub fn none() -> Self {
        Self {
            data: Mutex::new(None),
        }
    }

    pub fn update(&self, new: T) {
        match self.data.lock() {
            Ok(mut lock) => {
                *lock = Some(new);
            }
            Err(e) => {
                warn!("Failed to cache previous data as mutex was poisoned!");
            }
        }
    }

    pub fn get(&self) -> Option<T> {
        match self.data.lock() {
            Ok(lock) => lock.clone(),
            Err(e) => {
                warn!("Failed to retrieve previous data as mutex was posioned!");
                None
            }
        }
    }
}
