#![cfg(test)]

use super::{
    aliases::LiteLLMModelTable,
    archived::{LiteLLMDeletedTeamTable, LiteLLMTeamTableCachedObj},
    decoder::{MAX_ROW_BYTES, TeamRowError, decode_team_rows, parse_data},
    member::{BudgetLimitEntry, Member, MemberBase},
    membership::{LiteLLMTeamMembership, MembershipBudget},
    team::{LiteLLMTeamTable, TeamBase},
    validator::set_model_info,
};
use crate::budget_permission_rows::JsonData;
use rstest::rstest;
use rust_decimal::Decimal;
use serde::{Serialize, de::DeserializeOwned};
use std::{collections::BTreeMap, fs, path::PathBuf};

fn parse<T>(text: &str) -> T
where
    T: DeserializeOwned,
{
    serde_json::from_str(text).unwrap()
}
fn dump<T>(value: &T) -> JsonData
where
    T: Serialize,
{
    parse(&serde_json::to_string(value).unwrap())
}
fn team(text: &str) -> LiteLLMTeamTable {
    parse(text)
}
fn member(text: &str) -> LiteLLMTeamMembership {
    parse(text)
}
fn invalid<T>(text: &str) -> String
where
    T: DeserializeOwned,
{
    serde_json::from_str::<T>(text).err().unwrap().to_string()
}

#[rstest]
#[case("[]")]
#[case("null")]
#[case("1")]
#[case("\"text\"")]
fn member_requires_dict_exact_error(#[case] text: &str) {
    assert_eq!(
        MemberBase::try_from(parse::<JsonData>(text))
            .unwrap_err()
            .to_string(),
        "input needs to be a dictionary"
    );
}
#[rstest]
#[case("{}")]
#[case(r#"{"user_id":null,"user_email":null}"#)]
fn member_missing_and_null_identity_error(#[case] text: &str) {
    assert_eq!(
        invalid::<MemberBase>(text),
        "Either user id or user email must be provided"
    );
    assert_eq!(
        invalid::<Member>(text),
        "Either user id or user email must be provided"
    );
    assert!(
        invalid::<Member>(&format!(r#"{{"role":"user","user_id":{text}}}"#)).contains("string")
    );
}
#[test]
fn member_empty_identity_and_both_allowed() {
    let row: MemberBase = parse(r#"{"user_id":"","user_email":"A@EXAMPLE.ORG"}"#);
    assert_eq!(row.user_id.as_deref(), Some(""));
    assert_eq!(row.user_email.as_deref(), Some("A@EXAMPLE.ORG"));
    assert_eq!(parse::<MemberBase>(r#"{"user_email":""}"#).user_id, None);
}
#[rstest]
#[case("null", false)]
#[case("\"Admin\"", false)]
#[case("\"owner\"", false)]
#[case("1", false)]
#[case("\"admin\"", true)]
#[case("\"user\"", true)]
fn member_role_required_exact_literals(#[case] role: &str, #[case] valid: bool) {
    let text = format!(r#"{{"user_id":"u","role":{role}}}"#);
    assert_eq!(serde_json::from_str::<Member>(&text).is_ok(), valid);
    invalid::<Member>(r#"{"user_id":"u"}"#);
}

#[test]
fn budget_window_required_fields_and_no_duration_policy() {
    invalid::<BudgetLimitEntry>(r#"{"max_budget":1}"#);
    invalid::<BudgetLimitEntry>(r#"{"budget_duration":"x"}"#);
    let row: BudgetLimitEntry = parse(r#"{"budget_duration":"not a duration","max_budget":-2}"#);
    assert_eq!(row.max_budget, Decimal::from(-2_i32));
    assert_eq!(row.reset_at, None);
}
#[rstest]
#[case("{}")]
#[case(r#"{"created_by":"c"}"#)]
#[case(r#"{"updated_by":"u"}"#)]
fn alias_table_required_authors_and_string_preservation(#[case] text: &str) {
    invalid::<LiteLLMModelTable>(text);
    let row: LiteLLMModelTable =
        parse(r#"{"created_by":"c","updated_by":"u","model_aliases":"{bad"}"#);
    assert_eq!(
        serde_json::to_string(&row.model_aliases).unwrap(),
        r#""{bad""#
    );
    assert_eq!(row.id, None);
    assert_eq!(row.team, None);
}
#[test]
fn recursive_alias_team_projection() {
    let row = team(
        r#"{"team_id":"outer","litellm_model_table":{"created_by":"c","updated_by":"u","team":{"team_id":"inner","metadata":"{}"}}}"#,
    );
    let nested = row.fields.litellm_model_table.unwrap().team.unwrap();
    assert_eq!(nested.fields.base.team_id, "inner");
    assert!(nested.fields.base.metadata.unwrap().is_empty());
}
#[test]
fn team_base_all_defaults() {
    let row: TeamBase = parse("{}");
    let expected: JsonData = parse(include_str!("base-defaults.json"));
    assert_eq!(dump(&row), expected);
}
#[test]
fn team_required_id_and_all_added_defaults() {
    invalid::<LiteLLMTeamTable>("{}");
    invalid::<LiteLLMTeamTable>(r#"{"team_id":null}"#);
    let row = team(r#"{"team_id":"t"}"#);
    assert_eq!(team(include_str!("team-defaults.json")), row);
    assert_eq!(
        dump(&row),
        parse::<JsonData>(include_str!("team-defaults.json"))
    );
}
#[rstest]
#[case("admins")]
#[case("members")]
#[case("members_with_roles")]
#[case("models")]
#[case("blocked")]
fn team_all_explicit_fields_and_nullability(#[case] field: &str) {
    let text = include_str!("explicit.json");
    assert_eq!(dump(&team(text)), parse::<JsonData>(text));
    invalid::<TeamBase>(&format!(r#"{{"{field}":null}}"#));
    let row = team(
        r#"{"team_id":"t","model_spend":null,"model_max_budget":null,"allow_team_guardrail_config":null}"#,
    );
    assert_eq!(row.fields.model_spend.0, None);
    assert_eq!(row.fields.model_max_budget.0, None);
    assert_eq!(row.fields.allow_team_guardrail_config.0, None);
}
#[test]
fn inherited_prepass_runs_before_archive_fields() {
    let cached = invalid::<LiteLLMTeamTableCachedObj>(
        r#"{"team_id":"t","metadata":"bad","last_refreshed_at":"bad"}"#,
    );
    let deleted = invalid::<LiteLLMDeletedTeamTable>(
        r#"{"team_id":"t","metadata":"bad","deleted_at":"bad"}"#,
    );
    assert!(cached.contains("Field metadata should be a valid dictionary"));
    assert!(deleted.contains("Field metadata should be a valid dictionary"));
}
#[test]
fn cached_timestamp_is_not_a_monetary_amount() {
    let cached: LiteLLMTeamTableCachedObj = parse(r#"{"team_id":"t","last_refreshed_at":1e100}"#);
    assert_eq!(
        cached.last_refreshed_at.map(f64::to_bits),
        Some(1e100_f64.to_bits())
    );
}
#[test]
fn cached_timestamp_and_deleted_metadata() {
    let cached: LiteLLMTeamTableCachedObj = parse(r#"{"team_id":"t","last_refreshed_at":123.5}"#);
    assert_eq!(
        serde_json::to_string(&cached.last_refreshed_at).unwrap(),
        "123.5"
    );
    assert_eq!(
        parse::<LiteLLMTeamTableCachedObj>(r#"{"team_id":"t"}"#).last_refreshed_at,
        None
    );
    let deleted: LiteLLMDeletedTeamTable = parse(
        r#"{"team_id":"t","id":"d","deleted_at":"2026-01-01T00:00:00","deleted_by":"a","deleted_by_api_key":"key-id","litellm_changed_by":"c","policies":["p"]}"#,
    );
    assert_eq!(deleted.audit.deleted_by.as_deref(), Some("a"));
    assert_eq!(deleted.team.fields.policies, Some(vec!["p".to_owned()]));
    assert_eq!(deleted.audit.id.as_deref(), Some("d"));
}
#[rstest]
#[case("null")]
#[case(r#"{"x":1}"#)]
#[case("\"[]\"")]
fn team_empty_dict_roster_only(#[case] roster: &str) {
    assert!(
        team(r#"{"team_id":"t","members_with_roles":{}}"#)
            .fields
            .base
            .members_with_roles
            .is_empty()
    );
    invalid::<LiteLLMTeamTable>(&format!(
        r#"{{"team_id":"t","members_with_roles":{roster}}}"#
    ));
    let row = team(
        r#"{"team_id":"t","members_with_roles":[{"user_id":"a","role":"user"},{"user_id":"a","role":"user"}]}"#,
    );
    assert_eq!(row.fields.base.members_with_roles.len(), 2);
}
#[test]
fn team_base_has_no_string_prepass() {
    invalid::<TeamBase>(r#"{"metadata":"{}"}"#);
    invalid::<TeamBase>(r#"{"members_with_roles":{}}"#);
}
#[rstest]
#[case(0)]
#[case(1)]
#[case(2)]
#[case(3)]
#[case(4)]
#[case(5)]
#[case(6)]
#[case(7)]
fn team_all_eight_string_fields_and_error_order(#[case] position: usize) {
    let fields = [
        "metadata",
        "aliases",
        "config",
        "permissions",
        "model_max_budget",
        "model_aliases",
        "router_settings",
        "budget_limits",
    ];
    let object = fields
        .iter()
        .skip(position)
        .map(|key| ((*key).to_owned(), JsonData::String("{".to_owned())))
        .collect();
    let mut data = JsonData::Object(object);
    assert_eq!(
        set_model_info(&mut data).unwrap_err().to_string(),
        format!("Field {} should be a valid dictionary", fields[position])
    );
}

#[rstest]
#[case("aliases")]
#[case("config")]
#[case("permissions")]
#[case("model_aliases")]
fn ignored_extra_field_can_fail_json(#[case] field: &str) {
    assert!(
        invalid::<LiteLLMTeamTable>(&format!(r#"{{"team_id":"t","{field}":"bad"}}"#))
            .contains(&format!("Field {field}"))
    );
}
#[test]
fn valid_extra_fields_ignored() {
    assert_eq!(
        team(
            r#"{"team_id":"t","aliases":"1","config":"null","permissions":"[]","model_aliases":"true","unknown":"bad"}"#
        ),
        team(r#"{"team_id":"t"}"#)
    );
}
#[rstest]
#[case("1")]
#[case("[]")]
#[case("true")]
#[case("\"{}\"")]
fn model_spend_not_model_max_budget_decode(#[case] text: &str) {
    invalid::<LiteLLMTeamTable>(r#"{"team_id":"t","model_spend":"{}"}"#);
    assert_eq!(
        team(r#"{"team_id":"t","model_max_budget":"{}"}"#),
        team(r#"{"team_id":"t"}"#)
    );
    let encoded = serde_json::to_string(text).unwrap();
    invalid::<LiteLLMTeamTable>(&format!(r#"{{"team_id":"t","metadata":{encoded}}}"#));
}
#[test]
fn budget_limits_array_and_null() {
    let row = team(
        r#"{"team_id":"t","budget_limits":"[{\"budget_duration\":\"x\",\"max_budget\":-1}]"}"#,
    );
    assert_eq!(
        row.fields
            .base
            .budget_limits
            .unwrap()
            .first()
            .unwrap()
            .max_budget,
        Decimal::NEGATIVE_ONE
    );
    assert_eq!(
        team(r#"{"team_id":"t","budget_limits":"null"}"#)
            .fields
            .base
            .budget_limits,
        None
    );
}
#[test]
fn validator_mutates_prior_fields_before_error() {
    let mut row: JsonData = parse(r#"{"metadata":"{}","aliases":"bad","members_with_roles":{}}"#);
    set_model_info(&mut row).unwrap_err();
    assert_eq!(
        row,
        parse::<JsonData>(r#"{"metadata":{},"aliases":"bad","members_with_roles":[]}"#)
    );
}
#[test]
fn base_model_input_is_dumped() {
    let base: TeamBase = parse(r#"{"team_id":"t","metadata":{"ok":true}}"#);
    let mut row = dump(&base);
    set_model_info(&mut row).unwrap();
    let built = LiteLLMTeamTable::try_from(row).unwrap();
    assert_eq!(built.fields.base.metadata, base.metadata);
}
#[test]
fn fresh_defaults_and_unknown_fields() {
    let mut first = team(r#"{"team_id":"t"}"#);
    first.fields.base.admins.push("a".to_owned());
    first
        .fields
        .model_spend
        .0
        .as_mut()
        .unwrap()
        .insert("m".to_owned(), JsonData::Bool(true));
    let second = team(r#"{"team_id":"t","unknown":"ignored"}"#);
    assert!(second.fields.base.admins.is_empty());
    assert!(second.fields.model_spend.0.unwrap().is_empty());
}
#[rstest]
#[case("{}")]
#[case(r#"{"user_id":"u"}"#)]
#[case(r#"{"team_id":"t"}"#)]
fn membership_required_ids_and_nullable_counters(#[case] text: &str) {
    invalid::<LiteLLMTeamMembership>(text);
    let row = member(r#"{"user_id":"u","team_id":"t"}"#);
    assert_eq!(row.spend.0, Some(Decimal::ZERO));
    assert_eq!(row.total_spend.0, Some(Decimal::ZERO));
    assert_eq!(row.budget_id, None);
    assert_eq!(row.litellm_budget_table, None);
    let null = member(r#"{"user_id":"u","team_id":"t","spend":null,"total_spend":null}"#);
    assert_eq!(null.spend.0, None);
    assert_eq!(null.total_spend.0, None);
}
#[test]
fn full_budget_valid_variant() {
    let row = member(
        r#"{"user_id":"u","team_id":"t","litellm_budget_table":{"max_budget":1.5,"created_at":"2026-01-01T00:00:00","budget_reset_at":"2026-02-01T00:00:00"}}"#,
    );
    let MembershipBudget::Full(full) = row.litellm_budget_table.unwrap() else {
        panic!("full row lost");
    };
    assert!(full.budget_reset_at.is_some());
    assert_eq!(full.budget.max_budget, Some(Decimal::new(15, 1)));
}
#[rstest]
#[case(r#""created_at":"bad","budget_reset_at":"bad""#)]
#[case(r#""budget_reset_at":"2026-01-01T00:00:00""#)]
#[case(r#""created_at":"2026-01-01T00:00:00","budget_reset_at":"bad""#)]
fn invalid_full_budget_falls_back_to_base(#[case] fields: &str) {
    let row = member(&format!(
        r#"{{"user_id":"u","team_id":"t","litellm_budget_table":{{"rpm_limit":-1,{fields}}}}}"#
    ));
    assert!(matches!(
        row.litellm_budget_table,
        Some(MembershipBudget::Base(_))
    ));
    assert_eq!(row.safe_get_team_member_rpm_limit(), Some(-1));
}
#[rstest]
#[case("")]
#[case(r#", "created_at":"2026-01-01T00:00:00""#)]
fn member_limits_none_zero_negative(#[case] created: &str) {
    let absent = member(r#"{"user_id":"u","team_id":"t"}"#);
    assert_eq!(absent.safe_get_team_member_rpm_limit(), None);
    assert_eq!(absent.safe_get_team_member_tpm_limit(), None);
    let row = member(&format!(
        r#"{{"user_id":"u","team_id":"t","litellm_budget_table":{{"rpm_limit":0,"tpm_limit":-2{created}}}}}"#
    ));
    assert_eq!(row.safe_get_team_member_rpm_limit(), Some(0));
    assert_eq!(row.safe_get_team_member_tpm_limit(), Some(-2));
}
#[rstest]
#[case("NaN")]
#[case("Infinity")]
#[case("1e100")]
#[case("-1e100")]
fn team_money_nonfinite_and_overflow_rejected(#[case] amount: &str) {
    invalid::<LiteLLMTeamTable>(&format!(r#"{{"team_id":"t","max_budget":"{amount}"}}"#));
    invalid::<BudgetLimitEntry>(&format!(
        r#"{{"budget_duration":"x","max_budget":"{amount}"}}"#
    ));
    invalid::<LiteLLMTeamMembership>(&format!(
        r#"{{"user_id":"u","team_id":"t","spend":"{amount}"}}"#
    ));
}
#[rstest]
#[case(64)]
#[case(65)]
fn native_team_row_size_and_depth_limits(#[case] depth: usize) {
    let mut bytes = br#"{"kind":"team","row":{"team_id":"t"}}"#.to_vec();
    bytes.resize(MAX_ROW_BYTES, b' ');
    decode_team_rows(&bytes).unwrap();
    bytes.push(b' ');
    assert!(matches!(
        decode_team_rows(&bytes),
        Err(TeamRowError::TooLarge { .. })
    ));
    let text = format!("{}0{}", "[".repeat(depth), "]".repeat(depth));
    assert_eq!(parse_data(text.as_bytes()).is_ok(), depth == 64);
}
#[rstest]
#[case(64)]
#[case(65)]
fn native_nested_team_json_limits(#[case] depth: usize) {
    let text = format!("{}0{}", "[".repeat(depth), "]".repeat(depth));
    let mut row = parse::<JsonData>(&format!(
        r#"{{"aliases":{}}}"#,
        serde_json::to_string(&text).unwrap()
    ));
    assert_eq!(set_model_info(&mut row).is_ok(), depth == 64);
    let exact = format!("null{}", " ".repeat(MAX_ROW_BYTES.saturating_sub(4)));
    let mut exact_row = JsonData::Object(BTreeMap::from([(
        "metadata".to_owned(),
        JsonData::String(exact),
    )]));
    set_model_info(&mut exact_row).unwrap();
    assert_eq!(exact_row, parse::<JsonData>(r#"{"metadata":null}"#));
    let oversized = " ".repeat(MAX_ROW_BYTES.saturating_add(1));
    let mut large_row = parse::<JsonData>(&format!(
        r#"{{"metadata":{}}}"#,
        serde_json::to_string(&oversized).unwrap()
    ));
    assert!(matches!(
        set_model_info(&mut large_row),
        Err(TeamRowError::TooLarge { .. })
    ));
    let nested = format!(
        r#"{{"kind":"model","row":{{"created_by":"c","updated_by":"u","team":{{"team_id":"t","metadata":{}}}}}}}"#,
        serde_json::to_string(&format!("{}0{}", "[".repeat(65), "]".repeat(65))).unwrap()
    );
    assert!(matches!(
        decode_team_rows(nested.as_bytes()),
        Err(TeamRowError::TooDeep)
    ));
}
#[rstest]
fn replays_the_team_rows_corpus(#[files("fuzz/corpus/team_rows/*")] path: PathBuf) {
    let bytes = fs::read(&path).unwrap();
    let result = decode_team_rows(&bytes);
    if path
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .starts_with("valid-")
    {
        result.unwrap();
    } else {
        assert!(result.is_err(), "{}", path.display());
    }
}

use betterlite_postgres_fixture::connection::PostgresFixture;
use clap::Parser;
use secrecy::{ExposeSecret as _, SecretString};
use tokio::task::JoinSet;
#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}
async fn database() -> (
    PostgresFixture,
    tokio_postgres::Client,
    JoinSet<Result<(), tokio_postgres::Error>>,
) {
    let environment = Environment::parse_from(["team-tests"]);
    let url = environment.url.map(SecretString::from);
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
    client
        .batch_execute(include_str!("storage.sql"))
        .await
        .unwrap();
    (fixture, client, tasks)
}
#[tokio::test]
async fn postgres_team_model_defaults_differ_from_sql() {
    let (fixture, client, mut tasks) = database().await;
    let sql = client
        .query_one(
            r#"SELECT row_to_json(team)::TEXT FROM "LiteLLM_TeamTable" team WHERE team_id = 't'"#,
            &[],
        )
        .await
        .unwrap();
    let raw: JsonData = parse(sql.get(0));
    let JsonData::Object(fields) = raw else {
        panic!("not a row");
    };
    assert_eq!(fields.get("admins"), Some(&JsonData::Null));
    assert_eq!(
        fields.get("members_with_roles"),
        Some(&parse::<JsonData>("{}"))
    );
    invalid::<LiteLLMTeamTable>(sql.get(0));
    let stored = client.query_one(r#"SELECT row_to_json(team)::TEXT FROM "LiteLLM_TeamTable" team WHERE team_id = 'strings'"#, &[]).await.unwrap();
    let decoded = team(stored.get(0));
    assert_eq!(decoded.fields.spend, Some(Decimal::ZERO));
    assert_eq!(
        decoded.fields.base.metadata,
        Some(BTreeMap::from([(
            "stored".to_owned(),
            JsonData::Bool(true)
        )]))
    );
    let model = team(r#"{"team_id":"t"}"#);
    assert_eq!(model.fields.spend, None);
    assert_eq!(model.fields.created_at, None);
    assert!(decoded.fields.created_at.is_some());
    drop(client);
    tasks.join_next().await.unwrap().unwrap().unwrap();
    fixture.close_async().await.unwrap();
}
#[tokio::test]
async fn postgres_membership_budget_projection() {
    let (fixture, client, mut tasks) = database().await;
    let sql = client.query_one(r#"SELECT (to_jsonb(member) || jsonb_build_object('litellm_budget_table', to_jsonb(budget)))::TEXT FROM "LiteLLM_TeamMembership" member JOIN "LiteLLM_BudgetTable" budget USING (budget_id) WHERE member.user_id='u'"#, &[]).await.unwrap();
    let decoded = member(sql.get(0));
    assert!(matches!(
        decoded.litellm_budget_table,
        Some(MembershipBudget::Full(_))
    ));
    assert_eq!(decoded.spend.0, Some(Decimal::ZERO));
    assert_eq!(decoded.total_spend.0, Some(Decimal::ZERO));
    assert_eq!(decoded.safe_get_team_member_rpm_limit(), Some(0));
    assert_eq!(decoded.safe_get_team_member_tpm_limit(), Some(-2));
    drop(client);
    tasks.join_next().await.unwrap().unwrap().unwrap();
    fixture.close_async().await.unwrap();
}
