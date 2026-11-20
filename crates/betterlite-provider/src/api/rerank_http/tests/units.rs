use std::time::Duration;

use betterlite_models::decisions::Json;
use serde_json::Number;

use super::super::client::ClientKind;

#[test]
fn requested_timeout_renders_python_floats() {
    assert_eq!(super::super::convert::requested_timeout(None), "None");
    assert_eq!(
        super::super::convert::requested_timeout(Some(Duration::from_secs(5))),
        "5"
    );
    assert_eq!(
        super::super::convert::requested_timeout(Some(Duration::from_millis(1500))),
        "1.5"
    );
}

#[test]
fn elapsed_text_trims_the_fraction() {
    assert_eq!(super::super::convert::elapsed_text(Duration::ZERO), "0.0");
    assert_eq!(
        super::super::convert::elapsed_text(Duration::from_millis(100)),
        "0.1"
    );
    assert_eq!(
        super::super::convert::elapsed_text(Duration::from_millis(12_345)),
        "12.345"
    );
    assert_eq!(
        super::super::convert::elapsed_text(Duration::from_millis(1_200)),
        "1.2"
    );
}

#[test]
fn client_kind_needs_the_boolean_true() {
    assert_eq!(
        super::super::client::client_kind(&Json::Bool(true)),
        ClientKind::Asynchronous
    );
    assert_eq!(
        super::super::client::client_kind(&Json::Bool(false)),
        ClientKind::Sync
    );
    assert_eq!(
        super::super::client::client_kind(&Json::Number(Number::from(1_u8))),
        ClientKind::Sync
    );
    assert_eq!(
        super::super::client::client_kind(&Json::Null),
        ClientKind::Sync
    );
    assert_eq!(
        super::super::client::client_kind(&Json::Text(String::from("true"))),
        ClientKind::Sync
    );
}
