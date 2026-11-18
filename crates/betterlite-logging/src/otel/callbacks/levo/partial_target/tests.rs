#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_logging::otel::callbacks::levo::{LevoHealth, health_check};

fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[test]
fn levo_partial_without_the_organization_is_unhealthy_with_the_value_error_message() {
    let env = environment(&[
        ("LEVOAI_API_KEY", "levo-key"),
        ("LEVOAI_WORKSPACE_ID", "workspace-1"),
        (
            "LEVOAI_COLLECTOR_URL",
            "https://collector.levo.ai/v1/traces",
        ),
    ]);
    assert_eq!(
        health_check(&env),
        LevoHealth::Unhealthy {
            error_message: "LEVOAI_ORG_ID environment variable is required for Levo integration."
                .to_owned()
        }
    );
}

#[test]
fn levo_partial_with_a_blank_api_key_is_unhealthy_with_the_value_error_message() {
    let env = environment(&[
        ("LEVOAI_API_KEY", ""),
        ("LEVOAI_ORG_ID", "org-1"),
        ("LEVOAI_WORKSPACE_ID", "workspace-1"),
        (
            "LEVOAI_COLLECTOR_URL",
            "https://collector.levo.ai/v1/traces",
        ),
    ]);
    assert_eq!(
        health_check(&env),
        LevoHealth::Unhealthy {
            error_message: "LEVOAI_API_KEY environment variable is required for Levo integration."
                .to_owned()
        }
    );
}
