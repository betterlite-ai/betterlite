#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::{TlsPolicy, Verification};
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::pool::ClientPool;
use betterlite_roi::settings::ROISettings;

use super::client::GitHub;

fn tls() -> TlsPolicy {
    TlsPolicy {
        verification: Verification::Enabled,
        ca_file: None,
        certificate: None,
        http2: false,
        ecdh_curve: None,
        user_agent: "betterlite".to_owned(),
    }
}

#[test]
fn the_client_is_taken_from_the_pool_for_this_provider() {
    let pool = ClientPool::new(4, TransportLimits::DEFAULT);
    let routing = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap());
    let settings = ROISettings::default();
    let first = GitHub::pooled(&settings, &pool, &tls(), &routing).unwrap();
    let second = GitHub::pooled(&settings, &pool, &tls(), &routing).unwrap();
    assert_eq!(first.timeout, Duration::from_secs(45));
    assert_eq!(second.timeout, Duration::from_secs(45));
    assert_eq!(pool.client_count(), 1);
    first.close();
    let third = GitHub::pooled(&settings, &pool, &tls(), &routing).unwrap();
    assert_eq!(pool.client_count(), 1);
    second.close();
    third.close();
}

#[test]
fn a_pool_without_room_reports_the_pool_error() {
    let pool = ClientPool::new(0, TransportLimits::DEFAULT);
    let routing = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap());
    let settings = ROISettings::default();
    let error = GitHub::pooled(&settings, &pool, &tls(), &routing)
        .err()
        .unwrap();
    assert_eq!(
        error.to_string(),
        "the pool holds 0 clients, one per configured deployment policy, and has no room for another"
    );
}
