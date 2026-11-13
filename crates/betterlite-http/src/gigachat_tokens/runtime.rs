use std::time::{SystemTime, UNIX_EPOCH};

use uuid::Uuid;

pub trait GigachatRuntime: Send + Sync {
    fn now_millis(&self) -> i64;
    fn request_uid(&self) -> Uuid;
}

pub struct SystemRuntime;

impl GigachatRuntime for SystemRuntime {
    fn now_millis(&self) -> i64 {
        match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(elapsed) => i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX),
            Err(error) => {
                i64::try_from(error.duration().as_millis()).map_or(i64::MIN, i64::saturating_neg)
            }
        }
    }

    fn request_uid(&self) -> Uuid {
        Uuid::new_v4()
    }
}
