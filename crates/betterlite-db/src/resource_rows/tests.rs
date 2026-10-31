#![cfg(test)]

use super::access_group::AccessGroupTable;
use super::config::ConfigRow;
use super::credential::{
    CreateCredentialItem, CredentialBase, CredentialItem, UpdateCredentialItem,
};
use super::model::ProxyModelTable;
use super::organization::OrganizationTable;
use super::project::ProjectTable;
use super::{ResourceRowsError, decode_resource_rows};
use betterlite_postgres_fixture::connection::PostgresFixture;
use clap::Parser;
use rstest::rstest;
use rust_decimal::Decimal;
use secrecy::{ExposeSecret as _, SecretString};
use serde::de::DeserializeOwned;
use std::fs;
use std::path::PathBuf;
use tokio::task::JoinSet;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

fn postgres_test_url() -> Option<SecretString> {
    let environment = Environment::parse_from(["postgres-tests"]);
    environment.url.map(SecretString::from)
}

fn parse<T>(text: &str) -> T
where
    T: DeserializeOwned,
{
    serde_json::from_str(text).unwrap()
}
fn invalid<T>(text: &str) -> String
where
    T: DeserializeOwned,
{
    serde_json::from_str::<T>(text).err().unwrap().to_string()
}
fn access_group(text: &str) -> AccessGroupTable {
    parse(text)
}
fn config(text: &str) -> ConfigRow {
    parse(text)
}
fn model(text: &str) -> ProxyModelTable {
    parse(text)
}
fn organization(text: &str) -> OrganizationTable {
    parse(text)
}
fn project(text: &str) -> ProjectTable {
    parse(text)
}

#[test]
fn access_group_required_ids_and_all_defaults() {
    let row = access_group(r#"{"access_group_id":"g","access_group_name":"n"}"#);
    assert_eq!(row.access_group_id, "g");
    assert_eq!(row.access_group_name, "n");
    assert_eq!(row.description, None);
    assert_eq!(row.access_model_names, Vec::<String>::new());
    assert_eq!(row.access_mcp_server_ids, Vec::<String>::new());
    assert_eq!(row.access_agent_ids, Vec::<String>::new());
    assert_eq!(row.assigned_team_ids, Vec::<String>::new());
    assert_eq!(row.assigned_key_ids, Vec::<String>::new());
    assert_eq!(row.created_at, None);
    assert_eq!(row.created_by, None);
    assert_eq!(row.updated_at, None);
    assert_eq!(row.updated_by, None);
    invalid::<AccessGroupTable>(r#"{"access_group_name":"n"}"#);
    invalid::<AccessGroupTable>(r#"{"access_group_id":"g"}"#);
}

#[test]
fn access_group_all_fields_and_null_lists() {
    let row = access_group(
        r#"{"access_group_id":"g","access_group_name":"n","description":"d",
        "access_model_names":["m"],"access_mcp_server_ids":["s"],"access_agent_ids":["a"],
        "assigned_team_ids":["t"],"assigned_key_ids":["k"],"created_at":"2026-01-01T00:00:00Z",
        "created_by":"c","updated_at":"2026-01-02T00:00:00Z","updated_by":"u"}"#,
    );
    assert_eq!(row.description.as_deref(), Some("d"));
    assert_eq!(row.access_model_names, vec!["m".to_owned()]);
    assert_eq!(row.access_mcp_server_ids, vec!["s".to_owned()]);
    assert_eq!(row.access_agent_ids, vec!["a".to_owned()]);
    assert_eq!(row.assigned_team_ids, vec!["t".to_owned()]);
    assert_eq!(row.assigned_key_ids, vec!["k".to_owned()]);
    assert_eq!(row.created_by.as_deref(), Some("c"));
    assert_eq!(row.updated_by.as_deref(), Some("u"));
    invalid::<AccessGroupTable>(
        r#"{"access_group_id":"g","access_group_name":"n","access_model_names":null}"#,
    );
    invalid::<AccessGroupTable>(
        r#"{"access_group_id":"g","access_group_name":"n","access_mcp_server_ids":null}"#,
    );
    invalid::<AccessGroupTable>(
        r#"{"access_group_id":"g","access_group_name":"n","access_agent_ids":null}"#,
    );
    invalid::<AccessGroupTable>(
        r#"{"access_group_id":"g","access_group_name":"n","assigned_team_ids":null}"#,
    );
    invalid::<AccessGroupTable>(
        r#"{"access_group_id":"g","access_group_name":"n","assigned_key_ids":null}"#,
    );
}

#[test]
fn old_access_model_ids_not_an_alias() {
    let row = access_group(
        r#"{"access_group_id":"g","access_group_name":"n","access_model_ids":["old"]}"#,
    );
    assert_eq!(row.access_model_names, Vec::<String>::new());
}

#[test]
fn config_requires_dict_without_string_decode() {
    let row = config(r#"{"param_name":"p","param_value":{"a":1}}"#);
    assert_eq!(row.param_name, "p");
    invalid::<ConfigRow>(r#"{"param_name":"p","param_value":"{}"}"#);
    invalid::<ConfigRow>(r#"{"param_name":"p","param_value":[1]}"#);
    invalid::<ConfigRow>(r#"{"param_name":"p","param_value":1}"#);
    invalid::<ConfigRow>(r#"{"param_name":"p"}"#);
}

#[test]
fn credential_base_and_item_required_fields() {
    let base = parse::<CredentialBase>(r#"{"credential_name":"c","credential_info":{}}"#);
    assert_eq!(base.credential_name, "c");
    invalid::<CredentialBase>(r#"{"credential_name":"c"}"#);
    invalid::<CredentialBase>(r#"{"credential_info":{}}"#);
    let item = parse::<CredentialItem>(
        r#"{"credential_name":"c","credential_info":{},"credential_values":{}}"#,
    );
    assert_eq!(item.base.credential_name, "c");
    invalid::<CredentialItem>(r#"{"credential_name":"c","credential_info":{}}"#);
}

#[rstest]
#[case::model_id_only(r#"{"credential_name":"c","credential_info":{},"model_id":"m"}"#)]
#[case::values_only(
    r#"{"credential_name":"c","credential_info":{},"credential_values":{"k":"v"}}"#
)]
#[case::whitespace_model_id(r#"{"credential_name":"c","credential_info":{},"model_id":"  "}"#)]
fn credential_create_truthy_alternatives(#[case] text: &str) {
    parse::<CreateCredentialItem>(text);
}

#[test]
fn credential_create_both_allowed() {
    let row = parse::<CreateCredentialItem>(
        r#"{"credential_name":"c","credential_info":{},"credential_values":{"k":"v"},"model_id":"m"}"#,
    );
    assert_eq!(row.fields.model_id.as_deref(), Some("m"));
    assert!(row.fields.credential_values.is_some());
}

#[rstest]
#[case::missing(r#"{"credential_name":"c","credential_info":{}}"#)]
#[case::null(
    r#"{"credential_name":"c","credential_info":{},"credential_values":null,"model_id":null}"#
)]
#[case::empty(
    r#"{"credential_name":"c","credential_info":{},"credential_values":{},"model_id":""}"#
)]
fn credential_create_exact_missing_error(#[case] text: &str) {
    let value: super::JsonData = parse(text);
    assert_eq!(
        CreateCredentialItem::try_from(value)
            .unwrap_err()
            .to_string(),
        "Either credential_values or model_id must be set"
    );
}

#[test]
fn credential_create_wrong_shape_after_guard() {
    invalid::<CreateCredentialItem>(
        r#"{"credential_name":"c","credential_info":{},"credential_values":"nonempty"}"#,
    );
    invalid::<CreateCredentialItem>(
        r#"{"credential_name":"c","credential_info":{},"credential_values":["nonempty"]}"#,
    );
}

#[test]
fn credential_update_empty_and_null_allowed() {
    let row = parse::<UpdateCredentialItem>(
        r#"{"credential_name":"c","credential_info":{},"credential_values":null,"model_id":null}"#,
    );
    assert_eq!(row.credential_values, None);
    assert_eq!(row.model_id, None);
    let empty = parse::<UpdateCredentialItem>(
        r#"{"credential_name":"c","credential_info":{},"credential_values":{}}"#,
    );
    assert!(empty.credential_values.is_some());
    parse::<UpdateCredentialItem>(r#"{"credential_name":"c","credential_info":{}}"#);
}

#[test]
fn credential_payload_is_unmodified() {
    let item = parse::<CredentialItem>(
        r#"{"credential_name":"c","credential_info":{"nested":{"deep":[1,2,"x"]}},"credential_values":{"api_key":"ENC:abc=="},"unknown":"ignored"}"#,
    );
    let dumped = serde_json::to_value(&item).unwrap();
    assert_eq!(dumped["credential_info"]["nested"]["deep"][2], "x");
    assert_eq!(dumped["credential_values"]["api_key"], "ENC:abc==");
    assert!(dumped.get("unknown").is_none());
    let error = invalid::<CredentialItem>(r#"{"credential_name":"c"}"#);
    assert!(!error.contains("ENC:abc"));
}

#[test]
fn model_required_fields_and_all_defaults() {
    let row = model(r#"{"model_id":"m","model_name":"n","litellm_params":{}}"#);
    assert_eq!(row.fields.model_id, "m");
    assert_eq!(row.fields.model_name, "n");
    assert_eq!(row.fields.model_info, None);
    assert!(!row.fields.blocked);
    assert_eq!(row.fields.created_at, None);
    assert_eq!(row.fields.created_by, None);
    assert_eq!(row.fields.updated_at, None);
    assert_eq!(row.fields.updated_by, None);
    invalid::<ProxyModelTable>(r#"{"model_name":"n","litellm_params":{}}"#);
    invalid::<ProxyModelTable>(r#"{"model_id":"m","litellm_params":{}}"#);
    invalid::<ProxyModelTable>(r#"{"model_id":"m","model_name":"n"}"#);
}

#[test]
fn model_two_json_strings_decode_independently() {
    let row = model(
        r#"{"model_id":"m","model_name":"n","litellm_params":"{\"api_key\":\"k\"}","model_info":"{\"team_id\":\"t\"}"}"#,
    );
    assert!(row.fields.litellm_params.0.contains_key("api_key"));
    assert!(row.fields.model_info.unwrap().0.contains_key("team_id"));
}

#[test]
fn model_malformed_strings_fail_field_validation() {
    invalid::<ProxyModelTable>(r#"{"model_id":"m","model_name":"n","litellm_params":"{not json"}"#);
    invalid::<ProxyModelTable>(
        r#"{"model_id":"m","model_name":"n","litellm_params":{},"model_info":"{not json"}"#,
    );
}

#[test]
fn model_null_params_vs_null_info() {
    invalid::<ProxyModelTable>(r#"{"model_id":"m","model_name":"n","litellm_params":"null"}"#);
    let row = model(r#"{"model_id":"m","model_name":"n","litellm_params":{},"model_info":"null"}"#);
    assert_eq!(row.fields.model_info, None);
}

#[test]
fn model_scalar_array_and_double_encoded_text_rejected() {
    invalid::<ProxyModelTable>(r#"{"model_id":"m","model_name":"n","litellm_params":"[1,2]"}"#);
    invalid::<ProxyModelTable>(r#"{"model_id":"m","model_name":"n","litellm_params":"1"}"#);
    invalid::<ProxyModelTable>(
        r#"{"model_id":"m","model_name":"n","litellm_params":"\"{\\\"a\\\":1}\""}"#,
    );
}

#[test]
fn model_dict_unchanged() {
    let row = model(
        r#"{"model_id":"m","model_name":"n","litellm_params":{"api_key":"k"},"model_info":{"team_id":"t"}}"#,
    );
    assert!(row.fields.litellm_params.0.contains_key("api_key"));
    assert!(row.fields.model_info.unwrap().0.contains_key("team_id"));
}

#[test]
fn organization_required_fields_and_all_defaults() {
    let row = organization(r#"{"budget_id":"b","created_by":"c","updated_by":"u"}"#);
    assert_eq!(row.organization_id, None);
    assert_eq!(row.organization_alias, None);
    assert_eq!(row.spend, Decimal::ZERO);
    assert_eq!(row.metadata, None);
    assert_eq!(row.models, Vec::<String>::new());
    assert!(row.model_spend.0.is_some());
    assert_eq!(row.model_spend.0.unwrap().0.len(), 0);
    assert!(row.users.is_none());
    assert_eq!(row.litellm_budget_table, None);
    assert_eq!(row.object_permission, None);
    assert_eq!(row.object_permission_id, None);
    invalid::<OrganizationTable>(r#"{"created_by":"c","updated_by":"u"}"#);
    invalid::<OrganizationTable>(r#"{"budget_id":"b","updated_by":"u"}"#);
    invalid::<OrganizationTable>(r#"{"budget_id":"b","created_by":"c"}"#);
}

#[test]
fn organization_all_fields_nested_users() {
    let row = organization(
        r#"{"organization_id":"o","organization_alias":"a","budget_id":"b","spend":12.5,
        "metadata":{"k":"v"},"models":["gpt"],"model_spend":{"gpt":1.0},"created_by":"c",
        "updated_by":"u","object_permission_id":"p",
        "users":[{"user_id":"u1"}],
        "litellm_budget_table":{"budget_id":"b","max_budget":10},
        "object_permission":{"object_permission_id":"p"}}"#,
    );
    assert_eq!(row.organization_id.as_deref(), Some("o"));
    assert_eq!(row.spend, Decimal::new(125, 1));
    assert_eq!(row.models, vec!["gpt".to_owned()]);
    assert_eq!(row.users.unwrap().len(), 1);
    assert_eq!(
        row.litellm_budget_table.unwrap().budget_id.as_deref(),
        Some("b")
    );
    assert_eq!(row.object_permission.unwrap().object_permission_id, "p");
}

#[test]
fn nested_user_password_stays_excluded() {
    let row = organization(
        r#"{"budget_id":"b","created_by":"c","updated_by":"u","users":[{"user_id":"u1","password":"secret"}]}"#,
    );
    let dumped = serde_json::to_value(&row).unwrap();
    let user = &dumped["users"][0];
    assert!(user.get("password").is_none());
}

#[test]
fn project_required_id_and_all_defaults() {
    let row = project(r#"{"project_id":"p"}"#);
    assert_eq!(
        row,
        ProjectTable {
            project_id: "p".to_owned(),
            project_alias: None,
            description: None,
            team_id: None,
            budget_id: None,
            metadata: None,
            models: Vec::new(),
            spend: Decimal::ZERO,
            model_spend: None,
            model_rpm_limit: None,
            model_tpm_limit: None,
            blocked: false,
            object_permission_id: None,
            created_by: None,
            updated_by: None,
            created_at: None,
            updated_at: None,
            litellm_budget_table: None,
            object_permission: None,
        }
    );
    invalid::<ProjectTable>("{}");
}

#[test]
fn project_all_fields_nested_budget_and_permissions() {
    let row = project(
        r#"{"project_id":"p","project_alias":"a","description":"d","team_id":"t","budget_id":"b",
        "metadata":{"k":"v"},"models":["gpt"],"spend":3.5,"model_spend":{"gpt":1.0},
        "model_rpm_limit":{"gpt":10},"model_tpm_limit":{"gpt":100},"blocked":true,
        "object_permission_id":"op","created_by":"c","updated_by":"u",
        "created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-02T00:00:00Z",
        "litellm_budget_table":{"budget_id":"b","max_budget":10},
        "object_permission":{"object_permission_id":"op"}}"#,
    );
    assert_eq!(row.spend, Decimal::new(35, 1));
    assert!(row.blocked);
    assert_eq!(row.models, vec!["gpt".to_owned()]);
    assert!(row.model_spend.is_some());
    assert!(row.model_rpm_limit.is_some());
    assert!(row.model_tpm_limit.is_some());
    assert_eq!(
        row.litellm_budget_table.unwrap().budget_id.as_deref(),
        Some("b")
    );
    assert_eq!(row.object_permission.unwrap().object_permission_id, "op");
}

#[test]
fn organization_project_nulls_not_normalized() {
    invalid::<OrganizationTable>(
        r#"{"budget_id":"b","created_by":"c","updated_by":"u","spend":null}"#,
    );
    invalid::<OrganizationTable>(
        r#"{"budget_id":"b","created_by":"c","updated_by":"u","models":null}"#,
    );
    invalid::<ProjectTable>(r#"{"project_id":"p","spend":null}"#);
    invalid::<ProjectTable>(r#"{"project_id":"p","models":null}"#);
}

#[test]
fn fresh_defaults_and_unknown_fields() {
    let mut first = organization(r#"{"budget_id":"b","created_by":"c","updated_by":"u"}"#);
    first.models.push("one".to_owned());
    let second = organization(r#"{"budget_id":"b","created_by":"c","updated_by":"u"}"#);
    assert_eq!(second.models, Vec::<String>::new());
    assert_eq!(
        organization(r#"{"budget_id":"b","created_by":"c","updated_by":"u","unknown":1}"#)
            .budget_id,
        "b"
    );
    assert_eq!(project(r#"{"project_id":"p","unknown":1}"#).project_id, "p");
    assert_eq!(
        access_group(r#"{"access_group_id":"g","access_group_name":"n","unknown":1}"#)
            .access_group_id,
        "g"
    );
    assert_eq!(
        model(r#"{"model_id":"m","model_name":"n","litellm_params":{},"unknown":1}"#)
            .fields
            .model_id,
        "m"
    );
}

#[test]
fn scalar_coercion_and_timestamp_boundaries() {
    let naive = access_group(
        r#"{"access_group_id":"g","access_group_name":"n","created_at":"2026-01-01T00:00:00"}"#,
    );
    let aware = access_group(
        r#"{"access_group_id":"g","access_group_name":"n","created_at":"2026-01-01T01:00:00+01:00"}"#,
    );
    assert_eq!(
        naive.created_at.unwrap().utc(),
        aware.created_at.unwrap().utc()
    );
    invalid::<AccessGroupTable>(
        r#"{"access_group_id":"g","access_group_name":"n","access_model_names":"not-a-list"}"#,
    );
    invalid::<ProjectTable>(r#"{"project_id":"p","blocked":"not-a-bool"}"#);
}

#[test]
fn resource_money_nonfinite_and_overflow_rejected() {
    invalid::<OrganizationTable>(
        r#"{"budget_id":"b","created_by":"c","updated_by":"u","spend":1e1000}"#,
    );
    invalid::<ProjectTable>(r#"{"project_id":"p","spend":"NaN"}"#);
    invalid::<ProjectTable>(
        r#"{"project_id":"p","spend":"99999999999999999999999999999999999999999999999999999999"}"#,
    );
}

#[test]
fn native_resource_row_limits() {
    let mut exact =
        br#"{"kind":"access_group","row":{"access_group_id":"g","access_group_name":"n"}}"#
            .to_vec();
    exact.resize(0x0010_0000, b' ');
    decode_resource_rows(&exact).unwrap();
    let over = vec![b' '; 0x0010_0001];
    assert!(matches!(
        decode_resource_rows(&over),
        Err(ResourceRowsError::TooLarge { .. })
    ));
    let mut too_deep = "[".repeat(65);
    too_deep.push('0');
    too_deep.push_str(&"]".repeat(65));
    assert!(matches!(
        decode_resource_rows(too_deep.as_bytes()),
        Err(ResourceRowsError::TooDeep)
    ));
}

fn nested_object(depth: usize) -> String {
    let mut text = "{\"x\":".repeat(depth);
    text.push('0');
    text.push_str(&"}".repeat(depth));
    text
}

#[test]
fn native_nested_model_json_limits() {
    let at_limit = serde_json::to_string(&nested_object(63)).unwrap();
    let envelope = format!(
        r#"{{"kind":"model","row":{{"model_id":"m","model_name":"n","litellm_params":{at_limit}}}}}"#
    );
    decode_resource_rows(envelope.as_bytes()).unwrap();
    let over_limit = serde_json::to_string(&nested_object(64)).unwrap();
    let over_envelope = format!(
        r#"{{"kind":"model","row":{{"model_id":"m","model_name":"n","litellm_params":{over_limit}}}}}"#
    );
    // The nested TryFrom wraps TooDeep as a Json error carrying the same message.
    let error = decode_resource_rows(over_envelope.as_bytes()).unwrap_err();
    assert!(format!("{error:?}").contains("exceeds JSON depth 64"));
}

#[rstest]
fn replays_the_resource_rows_corpus(#[files("fuzz/corpus/resource_rows/*")] path: PathBuf) {
    drop(decode_resource_rows(&fs::read(path).unwrap()));
}

async fn seed_resource_rows(client: &tokio_postgres::Client) {
    client
        .batch_execute(
            r#"
            INSERT INTO "LiteLLM_BudgetTable" (budget_id, max_budget, created_by, updated_by)
                VALUES ('b', 10, 'test', 'test');
            INSERT INTO "LiteLLM_CredentialsTable"
                (credential_id, credential_name, credential_values, credential_info, created_by, updated_by)
                VALUES ('cr', 'name', '{"api_key":"ENC:abc=="}', '{}', 'test', 'test');
            INSERT INTO "LiteLLM_ProxyModelTable"
                (model_id, model_name, litellm_params, created_by, updated_by)
                VALUES ('m', 'gpt', '"{\"api_key\":\"ENC:xyz==\"}"', 'test', 'test');
            INSERT INTO "LiteLLM_OrganizationTable" (organization_id, organization_alias, budget_id, models, created_by, updated_by)
                VALUES ('o', 'org', 'b', ARRAY[]::TEXT[], 'test', 'test');
            INSERT INTO "LiteLLM_Config" (param_name, param_value) VALUES ('p', '{"k":"v"}');
            "#,
        )
        .await
        .unwrap();
}

async fn row_as<T>(client: &tokio_postgres::Client, query: &str) -> T
where
    T: DeserializeOwned,
{
    let text: String = client.query_one(query, &[]).await.unwrap().get(0);
    serde_json::from_str(&text).unwrap()
}

async fn connect_seeded_resource_rows() -> (
    tokio_postgres::Client,
    JoinSet<Result<(), tokio_postgres::Error>>,
    PostgresFixture,
) {
    let url = postgres_test_url();
    let fixture = PostgresFixture::provision_schema(url.as_ref())
        .resolve()
        .await
        .unwrap();
    let (client, connection) =
        tokio_postgres::connect(fixture.options.url.expose_secret(), tokio_postgres::NoTls)
            .await
            .unwrap();
    let mut tasks = JoinSet::new();
    tasks.spawn(connection);
    client
        .batch_execute(include_str!(
            "../../../../tools/postgres-fixture/fixtures/baseline.sql"
        ))
        .await
        .unwrap();
    seed_resource_rows(&client).await;
    (client, tasks, fixture)
}

async fn assert_resource_rows_raw_storage(client: &tokio_postgres::Client) {
    let credential: CredentialItem = row_as(
        client,
        r#"SELECT row_to_json(credential)::TEXT FROM "LiteLLM_CredentialsTable" credential WHERE credential_id = 'cr'"#,
    )
    .await;
    assert_eq!(
        credential.credential_values.0.get("api_key"),
        Some(&super::JsonData::String("ENC:abc==".to_owned()))
    );
    let model: ProxyModelTable = row_as(
        client,
        r#"SELECT row_to_json(model)::TEXT FROM "LiteLLM_ProxyModelTable" model WHERE model_id = 'm'"#,
    )
    .await;
    assert_eq!(
        model.fields.litellm_params.0.get("api_key"),
        Some(&super::JsonData::String("ENC:xyz==".to_owned()))
    );
    let organization: OrganizationTable = row_as(
        client,
        r#"SELECT row_to_json(organization)::TEXT FROM "LiteLLM_OrganizationTable" organization WHERE organization_id = 'o'"#,
    )
    .await;
    assert_eq!(organization.budget_id, "b");
    assert_eq!(organization.spend, Decimal::ZERO);
    let config: ConfigRow = row_as(
        client,
        r#"SELECT row_to_json(config)::TEXT FROM "LiteLLM_Config" config WHERE param_name = 'p'"#,
    )
    .await;
    assert_eq!(
        config.param_value.0.get("k"),
        Some(&super::JsonData::String("v".to_owned()))
    );
}

#[tokio::test]
async fn postgres_resource_rows_keep_raw_storage() {
    let (client, mut tasks, fixture) = connect_seeded_resource_rows().await;
    assert_resource_rows_raw_storage(&client).await;
    drop(client);
    tasks.join_next().await.unwrap().unwrap().unwrap();
    fixture.close_async().await.unwrap();
}
