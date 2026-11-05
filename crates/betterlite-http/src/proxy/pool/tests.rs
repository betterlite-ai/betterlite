#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::{TlsPolicy, Verification};
use betterlite_config::types::ProxyTrust;
use rstest::rstest;

use super::{ClientChoice, ClientPool, PoolError};
use crate::proxy::limits::TransportLimits;
use crate::proxy::tests::{default_client, kind};

fn tls(verification: Verification) -> TlsPolicy {
    TlsPolicy {
        verification,
        ca_file: None,
        certificate: None,
        http2: false,
        ecdh_curve: None,
        user_agent: "betterlite".to_owned(),
    }
}

fn outcome(selected: Result<ClientChoice, PoolError>) -> String {
    selected.map_or_else(|error| kind(&error), |choice| kind(&choice))
}

#[derive(Clone, Copy)]
enum Scenario {
    SuppliedMockBroken,
    MockBroken,
    Enabled,
    EnabledTwice,
    EnabledThenDisabled,
    Broken,
    DrainedThenDisabled,
}

#[rstest]
#[case::mock_after_supplied_client_async(1, Scenario::SuppliedMockBroken, "Supplied clients=0")]
#[case::mock_after_supplied_client_sync(1, Scenario::SuppliedMockBroken, "Supplied clients=0")]
#[case::mock_before_network_options_async(1, Scenario::MockBroken, "Mock clients=0")]
#[case::mock_before_network_options_sync(1, Scenario::MockBroken, "Mock clients=0")]
#[case::unsupplied_unmocked_selection_builds_a_pooled_client(
    1,
    Scenario::Enabled,
    "Pooled clients=1"
)]
#[case::the_same_policy_reuses_one_client(2, Scenario::EnabledTwice, "Pooled Pooled clients=1")]
#[case::different_tls_policies_stay_isolated(
    2,
    Scenario::EnabledThenDisabled,
    "Pooled Pooled clients=2"
)]
#[case::the_pool_is_bound_to_the_configured_deployments(
    1,
    Scenario::EnabledThenDisabled,
    "Pooled Full clients=1"
)]
#[case::an_unreadable_ca_file_is_an_error_not_a_client(1, Scenario::Broken, "Client clients=0")]
#[case::a_drained_pool_takes_no_new_policy(
    2,
    Scenario::DrainedThenDisabled,
    "Pooled drained Closed clients=1"
)]
#[tokio::test]
async fn http_pool_tests_behavior(
    #[case] capacity: usize,
    #[case] scenario: Scenario,
    #[case] expected: &str,
) {
    let pool = ClientPool::new(capacity, TransportLimits::DEFAULT);
    let routing = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap());
    let broken = TlsPolicy {
        ca_file: Some("/nonexistent/betterlite-ca.pem".to_owned()),
        ..tls(Verification::Enabled)
    };
    let enabled = tls(Verification::Enabled);
    let disabled = tls(Verification::Disabled);
    let observed = match scenario {
        Scenario::SuppliedMockBroken => {
            outcome(pool.select(Some(default_client()), true, &broken, &routing))
        }
        Scenario::MockBroken => outcome(pool.select(None, true, &broken, &routing)),
        Scenario::Enabled => outcome(pool.select(None, false, &enabled, &routing)),
        Scenario::EnabledTwice => {
            let first = outcome(pool.select(None, false, &enabled, &routing));
            format!(
                "{first} {}",
                outcome(pool.select(None, false, &enabled, &routing))
            )
        }
        Scenario::EnabledThenDisabled => {
            let first = outcome(pool.select(None, false, &enabled, &routing));
            format!(
                "{first} {}",
                outcome(pool.select(None, false, &disabled, &routing))
            )
        }
        Scenario::Broken => outcome(pool.select(None, false, &broken, &routing)),
        Scenario::DrainedThenDisabled => {
            let first = outcome(pool.select(None, false, &enabled, &routing));
            let client = pool.client(&enabled, &routing).unwrap();
            client.drain(Duration::from_secs(1)).await.unwrap();
            let refused = outcome(pool.select(None, false, &disabled, &routing));
            format!("{first} drained {refused}")
        }
    };
    assert_eq!(
        format!("{observed} clients={}", pool.client_count()),
        expected
    );
}
