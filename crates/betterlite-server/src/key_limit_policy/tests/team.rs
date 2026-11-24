use rstest::rstest;

use crate::key_fields::shared::hash_token_if_needed;
use crate::key_limit_policy::checks::check_team_key_limits;
use crate::key_limit_policy::error::{Entity, Rejection};
use crate::key_limit_policy::request::KeyLimitRequest;
use crate::team_parent_limits::error::RateLimit;

use super::support::{FakeReads, generate, keys, rejected, team, text_detail, update};

fn team_query() -> Vec<(&'static str, String)> {
    vec![("team_id", "team-1".to_owned())]
}

#[tokio::test]
async fn team_rpm_ceiling() {
    let team_table = team(r#"{"team_id": "team-1", "rpm_limit": 10}"#);
    let request = generate(r#"{"rpm_limit_type": "guaranteed_throughput", "rpm_limit": 6}"#);
    let reads = FakeReads::default().with_keys(keys(r#"[{"token": "k1", "rpm_limit": 5}]"#));
    let result = check_team_key_limits(&team_table, &KeyLimitRequest::from(&request), &reads).await;
    assert_eq!(
        text_detail(&rejected(result)),
        "Allocated RPM limit=5 + Key RPM limit=6 is greater than team RPM limit=10"
    );
    assert_eq!(*reads.queries.lock().unwrap(), team_query());
}

#[tokio::test]
async fn team_model_tpm() {
    let team_table =
        team(r#"{"team_id": "team-1", "metadata": {"model_tpm_limit": {"gpt-4": 10}}}"#);
    let request =
        generate(r#"{"rpm_limit_type": "guaranteed_throughput", "model_tpm_limit": {"gpt-4": 8}}"#);
    let reads = FakeReads::default().with_keys(keys(
        r#"[{"token": "k1", "metadata": {"model_tpm_limit": {"gpt-4": 5}}}]"#,
    ));
    let result = check_team_key_limits(&team_table, &KeyLimitRequest::from(&request), &reads).await;
    assert_eq!(
        text_detail(&rejected(result)),
        "Allocated TPM limit=5 + Key TPM limit=8 is greater than team TPM limit=10"
    );
}

#[rstest]
#[case::absent("{}")]
#[case::best_effort(r#"{"rpm_limit_type": "best_effort_throughput"}"#)]
#[tokio::test]
async fn team_non_guaranteed_skips(#[case] request: &str) {
    let team_table = team(r#"{"team_id": "team-1", "rpm_limit": 1}"#);
    let parsed = generate(request);
    let reads = FakeReads::default().with_keys(keys(r#"[{"token": "k1", "rpm_limit": 100}]"#));
    check_team_key_limits(&team_table, &KeyLimitRequest::from(&parsed), &reads)
        .await
        .unwrap();
    assert!(reads.queries.lock().unwrap().is_empty());
}

#[tokio::test]
async fn either_guaranteed_checks_both() {
    let team_table = team(r#"{"team_id": "team-1", "tpm_limit": 10}"#);
    let request = generate(r#"{"rpm_limit_type": "guaranteed_throughput", "tpm_limit": 6}"#);
    let reads = FakeReads::default().with_keys(keys(r#"[{"token": "k1", "tpm_limit": 5}]"#));
    let result = check_team_key_limits(&team_table, &KeyLimitRequest::from(&request), &reads).await;
    assert_eq!(
        text_detail(&rejected(result)),
        "Allocated TPM limit=5 + Key TPM limit=6 is greater than team TPM limit=10"
    );
}

#[tokio::test]
async fn team_metadata_type_ignored() {
    let team_table = team(r#"{"team_id": "team-1", "rpm_limit": 1}"#);
    let request =
        generate(r#"{"rpm_limit": 100, "metadata": {"rpm_limit_type": "guaranteed_throughput"}}"#);
    let reads = FakeReads::default().with_keys(keys(r#"[{"token": "k1", "rpm_limit": 100}]"#));
    check_team_key_limits(&team_table, &KeyLimitRequest::from(&request), &reads)
        .await
        .unwrap();
    assert!(reads.queries.lock().unwrap().is_empty());
}

#[tokio::test]
async fn team_update_excludes_self() {
    let hashed = hash_token_if_needed("sk-abc");
    let team_table = team(r#"{"team_id": "team-1", "rpm_limit": 10}"#);
    let request =
        update(r#"{"key": "sk-abc", "rpm_limit_type": "guaranteed_throughput", "rpm_limit": 10}"#);
    let reads = FakeReads::default().with_keys(keys(&format!(
        r#"[{{"token": "{hashed}", "rpm_limit": 10}}, {{"token": "k2", "rpm_limit": 0}}]"#
    )));
    check_team_key_limits(&team_table, &KeyLimitRequest::from(&request), &reads)
        .await
        .unwrap();
    let hashed_request = update(&format!(
        r#"{{"key": "{hashed}", "rpm_limit_type": "guaranteed_throughput", "rpm_limit": 10}}"#
    ));
    check_team_key_limits(&team_table, &KeyLimitRequest::from(&hashed_request), &reads)
        .await
        .unwrap();
}

#[tokio::test]
async fn team_update_no_alias_exclusion() {
    let team_table = team(r#"{"team_id": "team-1", "rpm_limit": 10}"#);
    let request = update(
        r#"{"key_alias": "alias-1", "rpm_limit_type": "guaranteed_throughput", "rpm_limit": 10}"#,
    );
    let reads = FakeReads::default().with_keys(keys(
        r#"[{"token": "k1", "key_alias": "alias-1", "rpm_limit": 10}]"#,
    ));
    let result = check_team_key_limits(&team_table, &KeyLimitRequest::from(&request), &reads).await;
    assert_eq!(
        text_detail(&rejected(result)),
        "Allocated RPM limit=10 + Key RPM limit=10 is greater than team RPM limit=10"
    );
}

#[tokio::test]
async fn team_model_before_scalar() {
    let team_table = team(
        r#"{"team_id": "team-1", "rpm_limit": 10, "metadata": {"model_rpm_limit": {"gpt-4": 10}}}"#,
    );
    let request = generate(
        r#"{"rpm_limit_type": "guaranteed_throughput", "rpm_limit": 10, "model_rpm_limit": {"gpt-4": 6}}"#,
    );
    let reads = FakeReads::default().with_keys(keys(
        r#"[{"token": "k1", "rpm_limit": 5, "metadata": {"model_rpm_limit": {"gpt-4": 5}}}]"#,
    ));
    let result = check_team_key_limits(&team_table, &KeyLimitRequest::from(&request), &reads).await;
    assert_eq!(
        rejected(result),
        Rejection::Allocation {
            limit: RateLimit::Rpm,
            entity: Entity::Team,
            allocated: 5,
            requested: 6,
            ceiling: 10,
        }
    );
}
