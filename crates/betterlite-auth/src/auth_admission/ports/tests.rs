#![cfg(test)]

use std::collections::BTreeMap;
use std::error::Error as _;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use indexmap::IndexMap;
use rstest::rstest;
use rust_decimal::Decimal;
use thiserror::Error;

use super::{DestinationSeedError, ManagedAdmission, TenantDestinations};
use crate::managed_admission::error::AdmissionError;
use crate::resolved_key::auth::{AuthInput, UserAPIKeyAuth};

fn empty_key() -> UserAPIKeyAuth {
    UserAPIKeyAuth::construct(AuthInput::Dict(BTreeMap::new())).unwrap()
}

struct MarkingAdmission;

#[async_trait]
impl ManagedAdmission for MarkingAdmission {
    async fn admit(&self, auth: &mut UserAPIKeyAuth) -> Result<(), AdmissionError> {
        auth.extra.requires_fresh_policy = true;
        Ok(())
    }

    async fn prepare(
        &self,
        auth: &mut UserAPIKeyAuth,
        target_name: &str,
        billable: bool,
    ) -> Result<(), AdmissionError> {
        auth.extra.invoked_agent_id = Some(target_name.to_owned());
        auth.extra.agent_invocation_cost = Some(if billable {
            Decimal::ONE
        } else {
            Decimal::ZERO
        });
        Ok(())
    }
}

struct RefusingAdmission {
    refusal: AdmissionError,
}

#[async_trait]
impl ManagedAdmission for RefusingAdmission {
    async fn admit(&self, _auth: &mut UserAPIKeyAuth) -> Result<(), AdmissionError> {
        Err(self.refusal.clone())
    }

    async fn prepare(
        &self,
        _auth: &mut UserAPIKeyAuth,
        _target_name: &str,
        _billable: bool,
    ) -> Result<(), AdmissionError> {
        Err(self.refusal.clone())
    }
}

#[derive(Default)]
struct RecordingDestinations {
    headers: Mutex<Vec<IndexMap<String, String>>>,
    end_users: Mutex<Vec<Option<String>>>,
}

impl TenantDestinations for RecordingDestinations {
    fn seed(
        &self,
        auth: &UserAPIKeyAuth,
        headers: &IndexMap<String, String>,
    ) -> Result<(), DestinationSeedError> {
        self.headers.lock().unwrap().push(headers.clone());
        self.end_users
            .lock()
            .unwrap()
            .push(auth.view.end_user_id.clone());
        Ok(())
    }
}

#[derive(Error, Debug)]
#[error("the tenant exporter could not be built")]
struct TenantExporterFailure;

struct FailingDestinations;

impl TenantDestinations for FailingDestinations {
    fn seed(
        &self,
        _auth: &UserAPIKeyAuth,
        _headers: &IndexMap<String, String>,
    ) -> Result<(), DestinationSeedError> {
        Err(DestinationSeedError {
            source: Box::new(TenantExporterFailure),
        })
    }
}

#[tokio::test]
async fn admit_changes_reach_the_caller() {
    let port: Arc<dyn ManagedAdmission> = Arc::new(MarkingAdmission);
    let mut auth = empty_key();
    assert!(!auth.extra.requires_fresh_policy);
    port.admit(&mut auth).await.unwrap();
    assert!(auth.extra.requires_fresh_policy);
}

#[rstest]
#[case::billable(true, Decimal::ONE)]
#[case::nonbillable(false, Decimal::ZERO)]
#[tokio::test]
async fn prepare_receives_the_target_and_billable_flag(
    #[case] billable: bool,
    #[case] cost: Decimal,
) {
    let port: Arc<dyn ManagedAdmission> = Arc::new(MarkingAdmission);
    let mut auth = empty_key();
    port.prepare(&mut auth, "managed-agent", billable)
        .await
        .unwrap();
    assert_eq!(
        auth.extra.invoked_agent_id.as_deref(),
        Some("managed-agent")
    );
    assert_eq!(auth.extra.agent_invocation_cost, Some(cost));
}

#[rstest]
#[case::forbidden(403, "the route is not open to agent identities")]
#[case::policy_unavailable(503, "Managed agent policy requires a database")]
#[tokio::test]
async fn admission_error_keeps_its_status_and_detail(#[case] status: u16, #[case] detail: &str) {
    let port: Arc<dyn ManagedAdmission> = Arc::new(RefusingAdmission {
        refusal: AdmissionError {
            status,
            detail: detail.to_owned(),
        },
    });
    let mut auth = empty_key();
    let error = port.admit(&mut auth).await.unwrap_err();
    assert_eq!(error.status, status);
    assert_eq!(error.detail, detail);
}

#[test]
fn seed_receives_the_auth_and_safe_headers() {
    let destinations = Arc::new(RecordingDestinations::default());
    let port: Arc<dyn TenantDestinations> = Arc::clone(&destinations);
    let mut auth = empty_key();
    auth.view.end_user_id = Some(String::from("tenant-user"));
    let mut headers = IndexMap::new();
    headers.insert(
        String::from("x-litellm-disable-callbacks"),
        String::from("langfuse"),
    );
    headers.insert(String::from("x-tenant"), String::from("acme"));
    port.seed(&auth, &headers).unwrap();
    assert_eq!(destinations.headers.lock().unwrap().first(), Some(&headers));
    assert_eq!(
        destinations.end_users.lock().unwrap().as_slice(),
        [Some(String::from("tenant-user"))]
    );
}

#[test]
fn destination_seed_error_shows_only_its_cause() {
    let port: Arc<dyn TenantDestinations> = Arc::new(FailingDestinations);
    let auth = empty_key();
    let error = port.seed(&auth, &IndexMap::new()).unwrap_err();
    assert_eq!(error.to_string(), TenantExporterFailure.to_string());
    let source = error.source().expect("the cause is kept");
    assert!(source.downcast_ref::<TenantExporterFailure>().is_some());
}
