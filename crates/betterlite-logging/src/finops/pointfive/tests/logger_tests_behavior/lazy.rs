use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use betterlite_config::sections::pointfive_params::PointFiveInitParams;
use secrecy::ExposeSecret as _;

use crate::finops::pointfive::logger::core::PointFiveCore;

use super::support::{FakeHttp, FixedSource, MutableEnvironment, client, service};

#[test]
fn each_access_rebuilds_the_client_from_the_current_environment() {
    let environment = MutableEnvironment::with(&[("POINTFIVE_API_KEY", "first-key")]);
    let http = FakeHttp::default();
    let source = Arc::new(service(&http, Arc::new(environment.clone())));
    let core = PointFiveCore::new(PointFiveInitParams::default(), None, source, false);
    let first = core
        .upload_client()
        .expect("the client builds")
        .api_key
        .expose_secret()
        .to_owned();
    environment.set("POINTFIVE_API_KEY", "second-key");
    let second = core
        .upload_client()
        .expect("the client builds")
        .api_key
        .expose_secret()
        .to_owned();
    assert_eq!(first, "first-key");
    assert_eq!(second, "second-key");
}

#[test]
fn a_url_edit_takes_effect_on_the_next_access() {
    let environment = MutableEnvironment::with(&[
        ("POINTFIVE_API_KEY", "the-key"),
        ("POINTFIVE_API_URL", "https://first.example/ingest"),
    ]);
    let http = FakeHttp::default();
    let source = Arc::new(service(&http, Arc::new(environment.clone())));
    let core = PointFiveCore::new(PointFiveInitParams::default(), None, source, false);
    let first = core.upload_client().expect("the client builds").api_url;
    environment.set("POINTFIVE_API_URL", "https://second.example/ingest");
    let second = core.upload_client().expect("the client builds").api_url;
    assert_eq!(first, "https://first.example/ingest");
    assert_eq!(second, "https://second.example/ingest");
}

#[test]
fn a_given_client_is_returned_without_asking_the_source() {
    let http = FakeHttp::default();
    let calls = Arc::new(AtomicUsize::new(0));
    let source = Arc::new(FixedSource {
        client: client(&http, 1),
        calls: Arc::clone(&calls),
    });
    let core = PointFiveCore::new(
        PointFiveInitParams::default(),
        Some(client(&http, 1)),
        source,
        false,
    );
    core.upload_client().expect("the client is given");
    core.upload_client().expect("the client is given");
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn a_missing_given_client_asks_the_source_on_every_access() {
    let http = FakeHttp::default();
    let calls = Arc::new(AtomicUsize::new(0));
    let source = Arc::new(FixedSource {
        client: client(&http, 1),
        calls: Arc::clone(&calls),
    });
    let core = PointFiveCore::new(PointFiveInitParams::default(), None, source, false);
    core.upload_client().expect("the source has a client");
    core.upload_client().expect("the source has a client");
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}
