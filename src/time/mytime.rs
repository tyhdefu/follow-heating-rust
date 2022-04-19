use chrono::{DateTime, Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc};

const FAKE_TIME: bool = false && cfg!(debug);

pub fn get_utc_time() -> DateTime<Utc> {
    if FAKE_TIME {
        return Utc::from_utc_datetime(&Utc, &get_naive_testing());
    }
    return Utc::now();
}

pub fn get_local_time() -> DateTime<Local> {
    if FAKE_TIME {
        let result = Local::from_local_datetime(&Local, &get_naive_testing());
        let first = result.earliest();
        if first.is_some() {
            return first.unwrap();
        }
        else {
            eprintln!("No localdatetime exists for {:?}", &get_naive_testing());
        }
    }
    return Local::now();
}

pub fn get_naive_testing() -> NaiveDateTime {
    let day = NaiveDate::from_ymd(2022, 04, 07);
    //if Local::now().minute() > 05 {
    //    return NaiveDateTime::new(
    //        day,
    //        NaiveTime::from_hms(04, 35, 00)
    //    );
    //}
    return NaiveDateTime::new(
        day,
        NaiveTime::from_hms(03, 30, 00)
    );
}