#![cfg(test)]

use std::collections::VecDeque;

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::completion_bridge::EventSource;

pub(super) struct Source {
    ids: VecDeque<Uuid>,
}

impl Default for Source {
    fn default() -> Self {
        Self {
            ids: (1..=10).map(Uuid::from_u128).collect(),
        }
    }
}

impl EventSource for Source {
    fn next_uuid(&mut self) -> Uuid {
        self.ids.pop_front().unwrap()
    }
    fn now(&mut self) -> DateTime<Utc> {
        DateTime::UNIX_EPOCH
    }
}

pub(super) fn uuid(number: u128) -> String {
    Uuid::from_u128(number).to_string()
}
