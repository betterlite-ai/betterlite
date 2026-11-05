#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};

use chrono::{DateTime, Utc};
use uuid::Uuid;

use super::wire::{Part, TaskResponse};
use crate::completion_bridge::EventSource;

pub(super) struct Source {
    ids: VecDeque<Uuid>,
}

impl Default for Source {
    fn default() -> Self {
        Self {
            ids: (1..=20).map(Uuid::from_u128).collect(),
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

pub(super) fn response(result: &str) -> TaskResponse {
    serde_json::from_str(&format!(r#"{{"result": {result}}}"#)).unwrap()
}

pub(super) fn text_part(text: &str) -> Part {
    Part {
        kind: Some("text".to_owned()),
        text: Some(text.to_owned()),
        extra: BTreeMap::new(),
    }
}
