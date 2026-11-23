#![cfg(test)]

use pyo3::prelude::*;

use super::fetch_card;
use crate::a2a::convert::{Settings, parse};

#[tokio::test]
async fn fetch_card_rejects_a_negative_timeout_before_the_request() {
    Python::initialize();
    let settings =
        parse::<Settings>(r#"{"base_url":"http://127.0.0.1:1","timeout":-1.0}"#).unwrap();
    assert!(fetch_card(settings).await.is_err());
}
