use std::sync::Arc;

use axum::http::StatusCode;
use rstest::rstest;

use super::super::responses::NO_DATABASE;
use super::support::{
    ErrorEnvelope, FakeFailure, RawTagBody, SpendPerTagBody, TagNamesBody, call, daily_tag,
    decimal, internal, raw_tag, tag_store,
};

const NO_DB_CONNECTED: &str =
    "No DB Connected. See - https://docs.litellm.ai/docs/proxy/virtual_keys";
const SPEND_TAGS_FAILED: &str = "/spend/tags Error";
const TAG_NAMES_FAILED: &str = "/spend/all_tag_names Error";

#[tokio::test]
async fn serves_raw_tag_counts_and_ignores_the_dates() {
    let mut fake = tag_store();
    fake.raw_tags = vec![raw_tag(Some("a"), 2, "3.5"), raw_tag(None, 1, "0.25")];
    let store = Arc::new(fake);
    let reply = call(
        "/spend/tags?start_date=not-a-date&end_date=also-bad",
        Some(&internal("u1")),
        Some(&store),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let rows: Vec<RawTagBody> = reply.decode();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].individual_request_tag.as_deref(), Some("a"));
    assert_eq!(rows[0].log_count, 2);
    assert_eq!(rows[0].total_spend, decimal("3.5"));
    assert_eq!(rows[1].individual_request_tag, None);
    assert_eq!(rows[1].total_spend, decimal("0.25"));
    assert!(reply.text().contains("\"total_spend\":3.5"));
    assert_eq!(store.calls(), vec!["raw_tag_spend".to_owned()]);
}

#[tokio::test]
async fn serves_every_distinct_tag_name() {
    let mut fake = tag_store();
    fake.names = Some(vec![Some("a".to_owned()), None, Some("b".to_owned())]);
    let store = Arc::new(fake);
    let reply = call(
        "/global/spend/all_tag_names",
        Some(&internal("u1")),
        Some(&store),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let body: TagNamesBody = reply.decode();
    assert_eq!(
        body.tag_names,
        vec![Some("a".to_owned()), None, Some("b".to_owned())]
    );
    assert!(reply.text().contains("null"));
}

#[tokio::test]
async fn serves_a_bare_list_for_a_missing_tag_name_result() {
    let mut fake = tag_store();
    fake.names = None;
    let store = Arc::new(fake);
    let reply = call(
        "/global/spend/all_tag_names",
        Some(&internal("u1")),
        Some(&store),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.text(), "[]");
}

#[rstest]
#[case::raw("/spend/tags", SPEND_TAGS_FAILED)]
#[case::tag_names("/global/spend/all_tag_names", TAG_NAMES_FAILED)]
#[case::daily(
    "/global/spend/tags?start_date=2024-01-01&end_date=2024-02-01",
    SPEND_TAGS_FAILED
)]
#[tokio::test]
async fn answers_a_tag_query_failure_through_the_proxy_boundary(
    #[case] uri: &str,
    #[case] prefix: &str,
) {
    let mut fake = tag_store();
    fake.failure = Some(FakeFailure::Undecodable);
    let store = Arc::new(fake);
    let reply = call(uri, Some(&internal("u1")), Some(&store)).await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    let envelope: ErrorEnvelope = reply.decode();
    assert_eq!(
        envelope.error.message,
        format!("{prefix}the stored row is not a JSON object")
    );
    assert_eq!(envelope.error.kind, "internal_error");
    assert_eq!(envelope.error.param.as_deref(), Some("None"));
    assert_eq!(envelope.error.code, "500");
}

#[rstest]
#[case::raw("/spend/tags", SPEND_TAGS_FAILED)]
#[case::tag_names("/global/spend/all_tag_names", TAG_NAMES_FAILED)]
#[case::daily("/global/spend/tags", SPEND_TAGS_FAILED)]
#[tokio::test]
async fn answers_a_missing_tag_database_through_the_proxy_boundary(
    #[case] uri: &str,
    #[case] prefix: &str,
) {
    let reply = call(uri, Some(&internal("u1")), None).await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    let envelope: ErrorEnvelope = reply.decode();
    assert_eq!(envelope.error.message, format!("{prefix}{NO_DATABASE}"));
    assert_eq!(envelope.error.code, "500");
}

#[rstest]
#[case::no_dates("/global/spend/tags")]
#[case::missing_end("/global/spend/tags?start_date=2024-01-01")]
#[case::missing_start("/global/spend/tags?end_date=2024-02-01")]
#[tokio::test]
async fn requires_both_dates_before_reading_the_view(#[case] uri: &str) {
    let mut fake = tag_store();
    fake.daily = vec![daily_tag(Some("a"), "1", 1)];
    let store = Arc::new(fake);
    let reply = call(uri, Some(&internal("u1")), Some(&store)).await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    let envelope: ErrorEnvelope = reply.decode();
    assert_eq!(
        envelope.error.message,
        "Please provide start_date and end_date"
    );
    assert_eq!(envelope.error.kind, "bad_request");
    assert_eq!(envelope.error.param, None);
    assert_eq!(envelope.error.code, "400");
    assert!(store.calls().is_empty());
}

fn expected_call(tags: Option<&[&str]>) -> String {
    tags.map_or_else(
        || "daily_tag_spend(s,e,None)".to_owned(),
        |list| {
            let owned: Vec<String> = list.iter().map(|tag| (*tag).to_owned()).collect();
            format!("daily_tag_spend(s,e,Some({owned:?}))")
        },
    )
}

#[rstest]
#[case::absent("", None)]
#[case::empty("&tags=", None)]
#[case::exact("&tags=all-tags", None)]
#[case::last("&tags=a,all-tags", None)]
#[case::first("&tags=all-tags,a", None)]
#[case::underscore("&tags=all_tags", Some(vec!["all_tags"]))]
#[case::capitalized("&tags=ALL-TAGS", Some(vec!["ALL-TAGS"]))]
#[case::pair("&tags=a,b", Some(vec!["a", "b"]))]
#[case::plus_space("&tags=a,+b", Some(vec!["a", " b"]))]
#[case::interior_empty("&tags=a,,b", Some(vec!["a", "", "b"]))]
#[case::only_commas("&tags=%2C", Some(vec!["", ""]))]
#[case::repeated("&tags=a&tags=b", Some(vec!["b"]))]
#[tokio::test]
async fn binds_only_the_tags_that_are_not_all_tags(
    #[case] suffix: &str,
    #[case] expected: Option<Vec<&str>>,
) {
    let mut fake = tag_store();
    fake.daily = vec![daily_tag(Some("a"), "1", 1)];
    let store = Arc::new(fake);
    let uri = format!("/global/spend/tags?start_date=s&end_date=e{suffix}");
    let reply = call(&uri, Some(&internal("u1")), Some(&store)).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(store.calls(), vec![expected_call(expected.as_deref())]);
}

#[tokio::test]
async fn passes_empty_date_text_to_the_store() {
    let store = Arc::new(tag_store());
    let reply = call(
        "/global/spend/tags?start_date=&end_date=",
        Some(&internal("u1")),
        Some(&store),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(store.calls(), vec!["daily_tag_spend(,,None)".to_owned()]);
}

#[tokio::test]
async fn passes_the_raw_date_text_to_the_store() {
    let store = Arc::new(tag_store());
    let reply = call(
        "/global/spend/tags?start_date=2024-1-5&end_date=not-a-date",
        Some(&internal("u1")),
        Some(&store),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        store.calls(),
        vec!["daily_tag_spend(2024-1-5,not-a-date,None)".to_owned()]
    );
}

#[tokio::test]
async fn sums_and_ranks_the_daily_rows_for_the_ui() {
    let mut fake = tag_store();
    fake.daily = vec![
        daily_tag(Some("b"), "1.5", 1),
        daily_tag(Some("a"), "1", 1),
        daily_tag(Some("a"), "2.5", 2),
        daily_tag(None, "0.75", 3),
    ];
    let store = Arc::new(fake);
    let reply = call(
        "/global/spend/tags?start_date=2024-01-01&end_date=2024-02-01",
        Some(&internal("u1")),
        Some(&store),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert!(reply.text().contains("\"spend\":3.5"));
    let body: SpendPerTagBody = reply.decode();
    assert_eq!(body.spend_per_tag.len(), 3);
    assert_eq!(body.spend_per_tag[0].name.as_deref(), Some("a"));
    assert_eq!(body.spend_per_tag[0].spend, decimal("3.5"));
    assert_eq!(body.spend_per_tag[0].log_count, 3);
    assert_eq!(body.spend_per_tag[1].name.as_deref(), Some("b"));
    assert_eq!(body.spend_per_tag[2].name, None);
    assert_eq!(body.spend_per_tag[2].spend, decimal("0.75"));
}

#[tokio::test]
async fn rounds_the_daily_tag_spend_to_four_places() {
    let mut fake = tag_store();
    fake.daily = vec![
        daily_tag(Some("even-tie"), "0.00005", 1),
        daily_tag(Some("odd-tie"), "0.00015", 1),
    ];
    let store = Arc::new(fake);
    let reply = call(
        "/global/spend/tags?start_date=2024-01-01&end_date=2024-02-01",
        Some(&internal("u1")),
        Some(&store),
    )
    .await;
    assert!(reply.text().contains("\"spend\":0.0002"));
    let body: SpendPerTagBody = reply.decode();
    assert_eq!(body.spend_per_tag[0].name.as_deref(), Some("odd-tie"));
    assert_eq!(body.spend_per_tag[0].spend, decimal("0.0002"));
    assert_eq!(body.spend_per_tag[1].name.as_deref(), Some("even-tie"));
    assert_eq!(body.spend_per_tag[1].spend, decimal("0"));
}

#[tokio::test]
async fn keeps_every_tag_past_the_top_ten() {
    let names: Vec<String> = (0_i32..12).map(|index| format!("tag-{index}")).collect();
    let mut fake = tag_store();
    fake.daily = names
        .iter()
        .zip(1_i64..)
        .map(|(name, spend)| daily_tag(Some(name.as_str()), &spend.to_string(), 1))
        .collect();
    let store = Arc::new(fake);
    let reply = call(
        "/global/spend/tags?start_date=2024-01-01&end_date=2024-02-01",
        Some(&internal("u1")),
        Some(&store),
    )
    .await;
    let body: SpendPerTagBody = reply.decode();
    assert_eq!(body.spend_per_tag.len(), 12);
}

#[tokio::test]
async fn answers_an_empty_daily_result_with_an_empty_list() {
    let store = Arc::new(tag_store());
    let reply = call(
        "/global/spend/tags?start_date=2024-01-01&end_date=2024-02-01",
        Some(&internal("u1")),
        Some(&store),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.text(), "{\"spend_per_tag\":[]}");
}

#[tokio::test]
async fn answers_a_daily_store_failure_with_the_connected_message() {
    let mut fake = tag_store();
    fake.failure = Some(FakeFailure::NoDatabase);
    let store = Arc::new(fake);
    let reply = call(
        "/global/spend/tags?start_date=2024-01-01&end_date=2024-02-01",
        Some(&internal("u1")),
        Some(&store),
    )
    .await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    let envelope: ErrorEnvelope = reply.decode();
    assert_eq!(
        envelope.error.message,
        format!("{SPEND_TAGS_FAILED}{NO_DB_CONNECTED}")
    );
}
