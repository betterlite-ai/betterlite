#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use betterlite_openai::containers::container_file_http::request::SslVerify;
use betterlite_openai::containers::container_file_http::transport::ClientSource;

use super::Source;

fn source() -> Source {
    let policy = ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap();
    Source {
        client: ProxiedClient::outbound(&Arc::new(policy)).unwrap(),
        deadline: Duration::from_secs(5),
    }
}

// the recorded deviation: the call's ssl_verify never reaches the client, which stays the session's own
#[test]
fn blocking_client_ignores_the_calls_ssl_verify() {
    let given = source();
    let plain = ClientSource::blocking_client(&given, None).unwrap();
    let refused = ClientSource::blocking_client(&given, Some(&SslVerify::Enabled(false))).unwrap();
    assert!(Arc::ptr_eq(&plain.client.policy, &refused.client.policy));
    assert_eq!(plain.deadline, refused.deadline);
    assert_eq!(plain.deadline, Duration::from_secs(5));
}

#[test]
fn async_client_ignores_the_calls_ssl_verify() {
    let given = source();
    let plain = ClientSource::async_client(&given, None).unwrap();
    let refused = ClientSource::async_client(&given, Some(&SslVerify::Enabled(false))).unwrap();
    assert!(Arc::ptr_eq(&plain.client.policy, &refused.client.policy));
    assert_eq!(plain.deadline, refused.deadline);
    assert_eq!(plain.deadline, Duration::from_secs(5));
}
