#![cfg(test)]

use secrecy::ExposeSecret;

use super::{SystemPrincipal, system_principal};
use crate::user_roles::UserRole;

#[test]
fn health_principal_all_fields() {
    let auth = system_principal(&SystemPrincipal::HealthCheck)
        .unwrap_or_else(|error| panic!("factory should validate: {error}"));
    assert_eq!(
        auth.extra
            .api_key
            .0
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("litellm-internal-health-check")
    );
    assert_eq!(
        auth.view.token.team_id.as_deref(),
        Some("litellm-internal-health-check")
    );
    assert_eq!(
        auth.view.token.key_alias.as_deref(),
        Some("litellm-internal-health-check")
    );
    assert_eq!(
        auth.view.team_alias.as_deref(),
        Some("litellm-internal-health-check")
    );
}

#[test]
fn jobs_principal_all_fields() {
    let auth = system_principal(&SystemPrincipal::Jobs)
        .unwrap_or_else(|error| panic!("factory should validate: {error}"));
    assert_eq!(
        auth.extra
            .api_key
            .0
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("litellm_internal_jobs")
    );
    assert_eq!(
        auth.view.token.key_alias.as_deref(),
        Some("litellm_internal_jobs")
    );
    assert_eq!(auth.view.token.team_id.as_deref(), Some("system"));
    assert_eq!(auth.view.team_alias.as_deref(), Some("system"));
    assert_eq!(auth.view.token.user_id.as_deref(), Some("system"));
    assert_eq!(auth.extra.user_role, Some(UserRole::ProxyAdmin));
}

#[test]
fn factories_not_virtual_key_provenance() {
    let health = system_principal(&SystemPrincipal::HealthCheck)
        .unwrap_or_else(|error| panic!("factory should validate: {error}"));
    let jobs = system_principal(&SystemPrincipal::Jobs)
        .unwrap_or_else(|error| panic!("factory should validate: {error}"));
    assert!(!health.extra.provenance.via_virtual_key);
    assert!(!jobs.extra.provenance.via_virtual_key);
}
