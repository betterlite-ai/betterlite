#![cfg(test)]

use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_core::api::video_status_transport::clients::{
    ClientFactories as _, SyncClient as _,
};
use betterlite_core::api::video_status_transport::error::{
    ErrorConverter as _, ProviderFailure, StatusFailure,
};
use betterlite_core::api::video_status_transport::records::{StatusMethod, StatusRequest};
use betterlite_http_fixture::reply::reply;
use betterlite_http_fixture::server::serve;
use indexmap::IndexMap;

use super::{NativeSource, StatusConverter, StatusFactories, user_agent};
use crate::video_calls::convert::SettingsWire;
use crate::video_calls::session::{VideoSession, config};

#[test]
fn configured_user_agent() {
    assert_eq!(
        user_agent(&BTreeMap::from([(
            "LITELLM_USER_AGENT".to_owned(),
            "video-agent".to_owned()
        )])),
        "video-agent"
    );
    assert_eq!(
        user_agent(&BTreeMap::new()),
        format!("betterlite/{}", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn synchronous_status_client() {
    let session = VideoSession::build(r#"{"environment":{}}"#).unwrap();
    let factories = StatusFactories {
        source: NativeSource::new(&session.environment, &session.client),
    };
    let serving = serve(reply("200 OK", &[], "status-bytes"), Duration::ZERO).unwrap();
    let client = factories.sync_client(Some(false)).unwrap();
    let response = client
        .send(&StatusRequest {
            method: StatusMethod::Get,
            url: format!("http://{}/status", serving.address),
            headers: IndexMap::from([("x-video".to_owned(), "present".to_owned())]),
            body: None,
        })
        .unwrap();
    assert_eq!(response.body, b"status-bytes");
    let request = serving.finish().unwrap();
    assert_eq!(request.line, "GET /status HTTP/1.1");
    assert!(request.head.contains("x-video: present"));
}

#[test]
fn status_error_facts() {
    let session = VideoSession::build(r#"{"environment":{}}"#).unwrap();
    let adapter = config(&session, &SettingsWire::default(), None);
    let failure = ProviderFailure {
        message: "upstream rejected".to_owned(),
        status_code: Some(429),
        headers: Some(IndexMap::from([("retry-after".to_owned(), "9".to_owned())])),
    };
    let error = StatusConverter.convert(&adapter, StatusFailure::Transform(&failure));
    assert_eq!(error.status_code, 429);
    assert_eq!(error.message, "upstream rejected");
    assert_eq!(
        error.headers.get("retry-after").map(String::as_str),
        Some("9")
    );
    assert!(!error.status_code_is_synthesized);
}
