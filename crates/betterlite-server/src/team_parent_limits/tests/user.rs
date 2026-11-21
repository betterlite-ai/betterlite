use std::sync::atomic::AtomicUsize;
use std::sync::{Arc, Mutex};

use betterlite_auth::auth_user_lookup::error::UserLookupError;
use betterlite_auth::auth_user_lookup::store::UserStore;
use betterlite_db::budget_permission_rows::JsonData;
use rstest::rstest;
use rust_decimal::Decimal;

use super::support::{
    RecordingCache, caller, close, count, database, error_text, lookup, new_team, rejected,
    user_store,
};
use crate::team_parent_limits::error::{Rejection, TeamLimitError};
use crate::team_parent_limits::request::TeamLimitRequest;
use crate::team_parent_limits::user::check_user_team_limits;

const CREATOR: &str = r#"{"user_id": "creator", "user_role": "internal_user", "models": ["gpt"], "tpm_limit": 100, "rpm_limit": 10}"#;

fn cached(max_budget: &str) -> Arc<RecordingCache> {
    let user = serde_json::from_str(&format!(
        r#"{{"user_id": "creator", "max_budget": {max_budget}}}"#
    ))
    .unwrap();
    Arc::new(RecordingCache {
        user: Some(user),
        reads: Mutex::new(Vec::new()),
        writes: AtomicUsize::new(0),
    })
}

async fn check(
    cache: &Arc<RecordingCache>,
    auth: &str,
    request: &str,
) -> Result<(), TeamLimitError> {
    let parsed = new_team(request);
    let users = lookup(cache, None);
    check_user_team_limits(&TeamLimitRequest::New(&parsed), &caller(auth), &users).await
}

fn role() -> String {
    "internal_user".to_owned()
}

const SEED_CREATOR: &str =
    r#"INSERT INTO "LiteLLM_UserTable" (user_id, max_budget) VALUES ('creator', 10);"#;
const USER_COUNT_SQL: &str = r#"SELECT count(*) FROM "LiteLLM_UserTable""#;

#[tokio::test]
async fn user_budget_lookup_no_upsert() {
    let (fixture, store) = database(SEED_CREATOR).await;
    let database_users: Arc<dyn UserStore> = user_store(&store);
    let cache = Arc::new(RecordingCache::default());
    let users = lookup(&cache, Some(Arc::clone(&database_users)));
    let over = new_team(r#"{"max_budget": 10.5}"#);
    let result =
        check_user_team_limits(&TeamLimitRequest::New(&over), &caller(CREATOR), &users).await;
    let rejection = rejected(result);
    assert_eq!(
        rejection,
        Rejection::UserBudget {
            ceiling: Decimal::from(10_i64),
            role: role()
        }
    );
    assert!(error_text(&rejection).contains("max budget"));
    let equal = new_team(r#"{"max_budget": 10}"#);
    check_user_team_limits(&TeamLimitRequest::New(&equal), &caller(CREATOR), &users)
        .await
        .unwrap();
    let ghost = caller(r#"{"user_id": "ghost"}"#);
    let missing = check_user_team_limits(&TeamLimitRequest::New(&equal), &ghost, &users).await;
    assert!(matches!(
        missing,
        Err(TeamLimitError::UserLookup(UserLookupError::NotFound(_)))
    ));
    assert_eq!(cache.read_keys(), ["creator", "creator", "ghost"]);
    assert_eq!(count(&store, USER_COUNT_SQL).await, 1);
    close(fixture, store).await;
}

#[rstest]
#[case::no_user_id(r#"{"user_role": "internal_user"}"#, r#"{"max_budget": 1000000}"#, 0)]
#[case::no_request_budget(CREATOR, r#"{"tpm_limit": 1}"#, 0)]
#[case::user_without_budget(CREATOR, r#"{"max_budget": 1000000}"#, 1)]
#[tokio::test]
async fn user_missing_budget_or_identity(
    #[case] auth: &str,
    #[case] request: &str,
    #[case] reads: usize,
) {
    let cache = cached("null");
    check(&cache, auth, request).await.unwrap();
    assert_eq!(cache.read_keys().len(), reads);
    assert_eq!(cache.write_count(), 0);
}

#[tokio::test]
async fn user_models_empty() {
    let cache = Arc::new(RecordingCache::default());
    let auth = r#"{"user_id": "creator", "models": []}"#;
    check(&cache, auth, r#"{"models": ["llama", "all-proxy-models"]}"#)
        .await
        .unwrap();
}

#[tokio::test]
async fn user_all_proxy_models_literal() {
    let cache = Arc::new(RecordingCache::default());
    let auth = r#"{"user_id": "creator", "models": ["all-proxy-models"]}"#;
    let rejection = rejected(check(&cache, auth, r#"{"models": ["gpt"]}"#).await);
    let allowed = vec![JsonData::String("all-proxy-models".to_owned())];
    assert_eq!(
        rejection,
        Rejection::UserModel {
            allowed,
            user: "creator".to_owned()
        }
    );
    error_text(&rejection);
    check(&cache, auth, r#"{"models": ["all-proxy-models"]}"#)
        .await
        .unwrap();
}

#[rstest]
#[case::equal(100, None)]
#[case::above(101, Some(100))]
#[tokio::test]
async fn user_tpm(#[case] requested: i64, #[case] ceiling: Option<i64>) {
    let cache = Arc::new(RecordingCache::default());
    let request = format!(r#"{{"tpm_limit": {requested}}}"#);
    let result = check(&cache, CREATOR, &request).await;
    match ceiling {
        None => result.unwrap(),
        Some(limit) => {
            let rejection = rejected(result);
            assert_eq!(
                rejection,
                Rejection::UserTpm {
                    ceiling: limit,
                    role: role()
                }
            );
            assert!(error_text(&rejection).contains("tpm limit"));
        }
    }
    check(&cache, r#"{"user_id": "creator"}"#, &request)
        .await
        .unwrap();
}

#[rstest]
#[case::equal(10, None)]
#[case::above(11, Some(10))]
#[tokio::test]
async fn user_rpm(#[case] requested: i64, #[case] ceiling: Option<i64>) {
    let cache = Arc::new(RecordingCache::default());
    let request = format!(r#"{{"rpm_limit": {requested}}}"#);
    let result = check(&cache, CREATOR, &request).await;
    match ceiling {
        None => result.unwrap(),
        Some(limit) => {
            let rejection = rejected(result);
            assert_eq!(
                rejection,
                Rejection::UserRpm {
                    ceiling: limit,
                    role: role()
                }
            );
            assert!(error_text(&rejection).contains("rpm limit"));
        }
    }
    check(&cache, r#"{"user_id": "creator"}"#, &request)
        .await
        .unwrap();
}

#[rstest]
#[case::budget_first(
    r#"{"max_budget": 11, "models": ["llama"], "tpm_limit": 101, "rpm_limit": 11}"#,
    "budget"
)]
#[case::models_second(
    r#"{"max_budget": 10, "models": ["llama"], "tpm_limit": 101, "rpm_limit": 11}"#,
    "models"
)]
#[case::tpm_third(
    r#"{"max_budget": 10, "models": ["gpt"], "tpm_limit": 101, "rpm_limit": 11}"#,
    "tpm"
)]
#[case::rpm_last(
    r#"{"max_budget": 10, "models": ["gpt"], "tpm_limit": 100, "rpm_limit": 11}"#,
    "rpm"
)]
#[tokio::test]
async fn user_failure_order(#[case] request: &str, #[case] first: &str) {
    let cache = cached("10");
    let rejection = rejected(check(&cache, CREATOR, request).await);
    let kind = match rejection {
        Rejection::UserBudget { .. } => "budget",
        Rejection::UserModel { .. } => "models",
        Rejection::UserTpm { .. } => "tpm",
        Rejection::UserRpm { .. } => "rpm",
        Rejection::OrganizationBudget { .. }
        | Rejection::OrganizationModel { .. }
        | Rejection::OrganizationTpm { .. }
        | Rejection::OrganizationRpm { .. }
        | Rejection::Allocation { .. } => "other",
    };
    assert_eq!(kind, first);
}
