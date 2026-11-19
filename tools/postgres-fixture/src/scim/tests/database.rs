use tokio_postgres::Client;

use crate::seed::FixtureSeed;
use crate::tests::postgres_url;

use super::support::{connect, disconnect, postgres_available, provision, sample_seed};

const USER_COLUMNS: &str = "SELECT \"created_at\"::text, \"updated_at\"::text, \"user_alias\", \"sso_user_id\", \"user_role\", \"teams\", \"metadata\"->>'origin', \"metadata\"->>'scim_managed' FROM \"LiteLLM_UserTable\" WHERE \"user_id\" = $1";

const UNTOUCHED_USER: &str = "SELECT \"user_alias\", \"sso_user_id\", \"user_role\" FROM \"LiteLLM_UserTable\" WHERE \"user_id\" = 'carol'";

const TEAM_COLUMNS: &str = "SELECT \"created_at\"::text, \"updated_at\"::text, jsonb_typeof(\"members_with_roles\"), jsonb_array_length(\"members_with_roles\"), \"members_with_roles\" #>> '{0,user_id}', \"members_with_roles\" #>> '{0,role}', \"members_with_roles\" #>> '{1,user_email}', \"members_with_roles\" #>> '{1,role}', \"members\", \"metadata\"->>'origin' FROM \"LiteLLM_TeamTable\" WHERE \"team_id\" = $1";

const KEY_COLUMNS: &str = "SELECT \"blocked\", jsonb_typeof(\"metadata\"), \"metadata\"->>'scim_blocked' FROM \"LiteLLM_VerificationToken\" WHERE \"token\" = $1";

#[tokio::test]
async fn scim_seed_writes_every_overlaid_column() {
    if !postgres_available() {
        return;
    }
    let admin_url = postgres_url();
    let seed = sample_seed();
    let fixture = provision(&admin_url, &seed).await;
    let (client, driver) = connect(&fixture.options.url).await;
    assert_user_columns(&client).await;
    assert_team_columns(&client).await;
    assert_key_columns(&client).await;
    disconnect(client, driver).await;
    fixture.close_async().await.unwrap();
}

async fn assert_user_columns(client: &Client) {
    let row = client
        .query_one(USER_COLUMNS, &[&"alice"])
        .await
        .expect("the alice overlay lands");
    assert_eq!(row.get::<_, String>(0), "2025-01-01 00:00:00");
    assert_eq!(row.get::<_, String>(1), "2025-01-02 00:00:00");
    assert_eq!(row.get::<_, Option<String>>(2).as_deref(), Some("Alice"));
    assert_eq!(row.get::<_, Option<String>>(3).as_deref(), Some(""));
    assert_eq!(
        row.get::<_, Option<String>>(4).as_deref(),
        Some("proxy_admin")
    );
    assert_eq!(
        row.get::<_, Option<Vec<String>>>(5),
        Some(vec!["team-red".to_owned(), "team-unseeded".to_owned()])
    );
    assert_eq!(row.get::<_, Option<String>>(6).as_deref(), Some("scim"));
    assert_eq!(row.get::<_, Option<String>>(7).as_deref(), Some("true"));
    let untouched = client
        .query_one(UNTOUCHED_USER, &[])
        .await
        .expect("the carol row stays");
    assert_eq!(untouched.get::<_, Option<String>>(0), None);
    assert_eq!(untouched.get::<_, Option<String>>(1), None);
    assert_eq!(untouched.get::<_, Option<String>>(2), None);
}

async fn assert_team_columns(client: &Client) {
    assert_red_team_columns(client).await;
    assert_blue_team_columns(client).await;
}

async fn assert_red_team_columns(client: &Client) {
    let row = client
        .query_one(TEAM_COLUMNS, &[&"team-red"])
        .await
        .expect("the team-red overlay lands");
    assert_eq!(row.get::<_, String>(0), "2025-03-01 00:00:00");
    assert_eq!(row.get::<_, String>(1), "2025-03-02 00:00:00");
    assert_eq!(row.get::<_, Option<String>>(2).as_deref(), Some("array"));
    assert_eq!(row.get::<_, Option<i32>>(3), Some(2_i32));
    assert_eq!(row.get::<_, Option<String>>(4).as_deref(), Some("alice"));
    assert_eq!(row.get::<_, Option<String>>(5).as_deref(), Some("admin"));
    assert_eq!(
        row.get::<_, Option<String>>(6).as_deref(),
        Some("roster@example.com")
    );
    assert_eq!(row.get::<_, Option<String>>(7).as_deref(), Some("user"));
    assert_eq!(
        row.get::<_, Option<Vec<String>>>(8),
        Some(vec!["alice".to_owned()])
    );
    assert_eq!(row.get::<_, Option<String>>(9).as_deref(), Some("scim"));
}

async fn assert_blue_team_columns(client: &Client) {
    let row = client
        .query_one(TEAM_COLUMNS, &[&"team-blue"])
        .await
        .expect("the team-blue overlay lands");
    assert_ne!(row.get::<_, String>(0), "2025-03-01 00:00:00");
    assert_eq!(row.get::<_, Option<String>>(2).as_deref(), Some("array"));
    assert_eq!(row.get::<_, Option<i32>>(3), Some(0_i32));
    assert_eq!(row.get::<_, Option<Vec<String>>>(8), None);
}

async fn assert_key_columns(client: &Client) {
    assert_eq!(
        key_state(client, "sk-alice-token").await,
        (
            Some(true),
            Some("object".to_owned()),
            Some("true".to_owned())
        )
    );
    assert_eq!(
        key_state(client, "sk-bob-token").await,
        (Some(false), Some("object".to_owned()), None)
    );
    assert_eq!(
        key_state(client, "sk-carol-token").await,
        (None, Some("object".to_owned()), None)
    );
}

async fn key_state(client: &Client, token: &str) -> (Option<bool>, Option<String>, Option<String>) {
    let hashed = FixtureSeed::key_hash(token);
    let row = client
        .query_one(KEY_COLUMNS, &[&hashed])
        .await
        .expect("the key overlay lands");
    (
        row.get::<_, Option<bool>>(0),
        row.get::<_, Option<String>>(1),
        row.get::<_, Option<String>>(2),
    )
}
