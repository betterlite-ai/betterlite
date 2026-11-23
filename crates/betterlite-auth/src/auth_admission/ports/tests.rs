#![cfg(test)]

use std::collections::BTreeMap;
use std::error::Error as _;
use std::sync::{Arc, Mutex, PoisonError};

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

fn assert_send_sync<T>()
where
    T: Send + Sync + ?Sized,
{
}

struct RecordingAdmission {
    refusal: Option<AdmissionError>,
}

impl RecordingAdmission {
    fn refuse(&self) -> Result<(), AdmissionError> {
        self.refusal
            .as_ref()
            .map_or(Ok(()), |refusal| Err(refusal.clone()))
    }
}

#[async_trait]
impl ManagedAdmission for RecordingAdmission {
    async fn admit(&self, auth: &mut UserAPIKeyAuth) -> Result<(), AdmissionError> {
        self.refuse()?;
        auth.extra.requires_fresh_policy = true;
        Ok(())
    }

    async fn prepare(
        &self,
        auth: &mut UserAPIKeyAuth,
        target_name: &str,
        billable: bool,
    ) -> Result<(), AdmissionError> {
        self.refuse()?;
        auth.extra.invoked_agent_id = Some(target_name.to_owned());
        auth.extra.agent_invocation_cost = Some(if billable {
            Decimal::ONE
        } else {
            Decimal::ZERO
        });
        Ok(())
    }
}

#[derive(Default)]
struct SeedRecords {
    headers: Mutex<Vec<IndexMap<String, String>>>,
    end_users: Mutex<Vec<Option<String>>>,
}

struct RecordingDestinations {
    failing: bool,
    records: Arc<SeedRecords>,
}

impl TenantDestinations for RecordingDestinations {
    fn seed(
        &self,
        auth: &UserAPIKeyAuth,
        headers: &IndexMap<String, String>,
    ) -> Result<(), DestinationSeedError> {
        if self.failing {
            return Err(DestinationSeedError {
                source: Box::new(TenantExporterFailure),
            });
        }
        self.records
            .headers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(headers.clone());
        self.records
            .end_users
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(auth.view.end_user_id.clone());
        Ok(())
    }
}

#[derive(Error, Debug)]
#[error("the tenant exporter could not be built")]
struct TenantExporterFailure;

#[tokio::test]
async fn admit_changes_reach_the_caller() {
    let port: Arc<dyn ManagedAdmission> = Arc::new(RecordingAdmission { refusal: None });
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
    let port: Arc<dyn ManagedAdmission> = Arc::new(RecordingAdmission { refusal: None });
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
    let refusal = AdmissionError {
        status,
        detail: detail.to_owned(),
    };
    let port: Arc<dyn ManagedAdmission> = Arc::new(RecordingAdmission {
        refusal: Some(refusal),
    });
    let mut auth = empty_key();
    let error = port.admit(&mut auth).await.unwrap_err();
    assert_eq!(error.status, status);
    assert_eq!(error.detail, detail);
}

#[test]
fn seed_receives_the_auth_and_safe_headers() {
    let records = Arc::new(SeedRecords::default());
    let port: Arc<dyn TenantDestinations> = Arc::new(RecordingDestinations {
        failing: false,
        records: Arc::clone(&records),
    });
    let mut auth = empty_key();
    auth.view.end_user_id = Some(String::from("tenant-user"));
    let mut headers = IndexMap::new();
    headers.insert(
        String::from("x-litellm-disable-callbacks"),
        String::from("langfuse"),
    );
    headers.insert(String::from("x-tenant"), String::from("acme"));
    port.seed(&auth, &headers).unwrap();
    assert_eq!(records.headers.lock().unwrap().first(), Some(&headers));
    assert_eq!(
        records.end_users.lock().unwrap().as_slice(),
        [Some(String::from("tenant-user"))]
    );
}

#[test]
fn ports_are_object_safe_and_send_and_sync() {
    assert_send_sync::<dyn ManagedAdmission>();
    assert_send_sync::<dyn TenantDestinations>();
}

#[test]
fn destination_seed_error_shows_only_its_cause() {
    let port: Arc<dyn TenantDestinations> = Arc::new(RecordingDestinations {
        failing: true,
        records: Arc::new(SeedRecords::default()),
    });
    let auth = empty_key();
    let error = port.seed(&auth, &IndexMap::new()).unwrap_err();
    assert_eq!(error.to_string(), TenantExporterFailure.to_string());
    let source = error.source().expect("the cause is kept");
    assert!(source.downcast_ref::<TenantExporterFailure>().is_some());
}
