use betterlite_db::resource_rows::organization::OrganizationTable;

use crate::key_fields::shared::hash_token_if_needed;
use crate::key_limit_policy::checks::check_org_key_limits;
use crate::key_limit_policy::error::KeyLimitError;
use crate::key_limit_policy::request::KeyLimitRequest;

use super::support::{FakeReads, generate, keys, organization, rejected, text_detail, update};

fn org(metadata: &str, budget: &str) -> OrganizationTable {
    organization(&format!(
        r#"{{"organization_id": "org-1", "budget_id": "budget-1", "created_by": "admin", "updated_by": "admin", "models": [], "metadata": {metadata}, "litellm_budget_table": {budget}}}"#
    ))
}

fn org_query() -> Vec<(&'static str, String)> {
    vec![("organization_id", "org-1".to_owned())]
}

#[tokio::test]
async fn org_model_limits() {
    let organization = org("{}", r#"{"budget_id": "budget-1", "rpm_limit": 20}"#);
    let request = generate(
        r#"{"tpm_limit_type": "guaranteed_throughput", "model_rpm_limit": {"gpt-4": 15}}"#,
    );
    let reads = FakeReads::default().with_keys(keys(
        r#"[{"token": "k1", "metadata": {"model_rpm_limit": {"gpt-4": 6}}}]"#,
    ));
    let result =
        check_org_key_limits(&organization, &KeyLimitRequest::from(&request), &reads).await;
    assert_eq!(
        text_detail(&rejected(result)),
        "Allocated RPM limit=6 + Key RPM limit=15 is greater than organization RPM limit=20"
    );
    assert_eq!(*reads.queries.lock().unwrap(), org_query());
}

#[tokio::test]
async fn org_type_metadata() {
    let organization = org("{}", r#"{"budget_id": "budget-1", "rpm_limit": 10}"#);
    let request =
        generate(r#"{"rpm_limit": 10, "metadata": {"rpm_limit_type": "guaranteed_throughput"}}"#);
    let reads = FakeReads::default().with_keys(keys(r#"[{"token": "k1", "rpm_limit": 10}]"#));
    let result =
        check_org_key_limits(&organization, &KeyLimitRequest::from(&request), &reads).await;
    assert_eq!(
        text_detail(&rejected(result)),
        "Allocated RPM limit=10 + Key RPM limit=10 is greater than organization RPM limit=10"
    );
    assert_eq!(*reads.queries.lock().unwrap(), org_query());
}

#[tokio::test]
async fn org_table_metadata_type_ignored() {
    let organization = org(
        r#"{"rpm_limit_type": "guaranteed_throughput"}"#,
        r#"{"budget_id": "budget-1", "rpm_limit": 1}"#,
    );
    let request = generate(r#"{"rpm_limit": 100}"#);
    let reads = FakeReads::default().with_keys(keys(r#"[{"token": "k1", "rpm_limit": 100}]"#));
    check_org_key_limits(&organization, &KeyLimitRequest::from(&request), &reads)
        .await
        .unwrap();
    assert!(reads.queries.lock().unwrap().is_empty());
}

#[tokio::test]
async fn org_declared_type_wins() {
    let organization = org(
        r#"{"rpm_limit_type": "guaranteed_throughput"}"#,
        r#"{"budget_id": "budget-1", "rpm_limit": 1}"#,
    );
    let request = generate(r#"{"rpm_limit_type": "best_effort_throughput", "rpm_limit": 100}"#);
    let reads = FakeReads::default().with_keys(keys(r#"[{"token": "k1", "rpm_limit": 100}]"#));
    check_org_key_limits(&organization, &KeyLimitRequest::from(&request), &reads)
        .await
        .unwrap();
    assert!(reads.queries.lock().unwrap().is_empty());
}

#[tokio::test]
async fn org_missing_budget() {
    let organization = org("{}", "null");
    let request = generate(
        r#"{"tpm_limit_type": "guaranteed_throughput", "tpm_limit": 1000, "rpm_limit": 1000}"#,
    );
    let reads = FakeReads::default().with_keys(keys(r#"[{"token": "k1", "rpm_limit": 100}]"#));
    check_org_key_limits(&organization, &KeyLimitRequest::from(&request), &reads)
        .await
        .unwrap();
}

#[tokio::test]
async fn org_metadata_models() {
    let organization = org(r#"{"model_tpm_limit": {"gpt-4": 5}}"#, "null");
    let request =
        generate(r#"{"rpm_limit_type": "guaranteed_throughput", "model_tpm_limit": {"gpt-4": 4}}"#);
    let reads = FakeReads::default().with_keys(keys(
        r#"[{"token": "k1", "metadata": {"model_tpm_limit": {"gpt-4": 2}}}]"#,
    ));
    let result =
        check_org_key_limits(&organization, &KeyLimitRequest::from(&request), &reads).await;
    assert_eq!(
        text_detail(&rejected(result)),
        "Allocated TPM limit=2 + Key TPM limit=4 is greater than organization TPM limit=5"
    );
}

#[tokio::test]
async fn org_update_excludes_self() {
    let hashed = hash_token_if_needed("sk-abc");
    let organization = org("{}", r#"{"budget_id": "budget-1", "rpm_limit": 10}"#);
    let request =
        update(r#"{"key": "sk-abc", "rpm_limit_type": "guaranteed_throughput", "rpm_limit": 10}"#);
    let reads = FakeReads::default().with_keys(keys(&format!(
        r#"[{{"token": "{hashed}", "rpm_limit": 10}}, {{"token": "k2", "rpm_limit": 0}}]"#
    )));
    check_org_key_limits(&organization, &KeyLimitRequest::from(&request), &reads)
        .await
        .unwrap();
}

#[tokio::test]
async fn org_missing_organization_id() {
    let organization = organization(
        r#"{"budget_id": "budget-1", "created_by": "admin", "updated_by": "admin", "models": [], "litellm_budget_table": {"budget_id": "budget-1", "rpm_limit": 10}}"#,
    );
    let request = generate(r#"{"rpm_limit_type": "guaranteed_throughput", "rpm_limit": 1}"#);
    let reads = FakeReads::default();
    let result =
        check_org_key_limits(&organization, &KeyLimitRequest::from(&request), &reads).await;
    assert!(matches!(result, Err(KeyLimitError::MissingOrganization)));
    assert!(reads.queries.lock().unwrap().is_empty());
}

#[tokio::test]
async fn org_model_before_scalar() {
    let organization = org("{}", r#"{"budget_id": "budget-1", "rpm_limit": 10}"#);
    let request = generate(
        r#"{"rpm_limit_type": "guaranteed_throughput", "rpm_limit": 10, "model_rpm_limit": {"gpt-4": 6}}"#,
    );
    let reads = FakeReads::default().with_keys(keys(
        r#"[{"token": "k1", "rpm_limit": 5, "metadata": {"model_rpm_limit": {"gpt-4": 5}}}]"#,
    ));
    let result =
        check_org_key_limits(&organization, &KeyLimitRequest::from(&request), &reads).await;
    assert_eq!(
        text_detail(&rejected(result)),
        "Allocated RPM limit=5 + Key RPM limit=6 is greater than organization RPM limit=10"
    );
}
