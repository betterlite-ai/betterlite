#[test]
fn json_field_returns_an_object_as_it_is() {
    let fields: JsonObject = serde_json::from_str(r#"{"guardrail":"generic_guardrail_api"}"#)
        .expect("the fixture is an object");
    let column = ColumnJson::Object(fields.clone());
    assert_eq!(parse_json_field(Some(&column)), Some(fields));
}

#[test]
fn json_field_parses_json_text_into_an_object() {
    let column = ColumnJson::Text(r#"{"guardrail":"generic_guardrail_api"}"#.to_owned());
    let parsed = parse_json_field(Some(&column)).expect("the text parses");
    assert_eq!(
        parsed.get("guardrail").and_then(JsonValue::as_str),
        Some("generic_guardrail_api")
    );
}

#[test]
fn json_field_gives_none_for_null() {
    assert_eq!(parse_json_field(None), None);
    let column = ColumnJson::Text("null".to_owned());
    assert_eq!(parse_json_field(Some(&column)), None);
}

#[test]
fn json_field_gives_none_for_bad_text() {
    let column = ColumnJson::Text("{not json".to_owned());
    assert_eq!(parse_json_field(Some(&column)), None);
}

#[rstest]
#[case::array("[1,2]")]
#[case::string("\"text\"")]
#[case::number("42")]
#[case::boolean("true")]
fn json_field_gives_none_for_a_non_object(#[case] stored: &str) {
    let column = ColumnJson::Text(stored.to_owned());
    assert_eq!(parse_json_field(Some(&column)), None, "{stored}");
}

#[test]
fn item_masks_sensitive_params() {
    let mut stored = row("g1", "guard", Some("pending_review"), Some("team-a"));
    stored.litellm_params = Some(ColumnJson::Text(
        r#"{"api_key":"secret-key-1234","api_base":"https://guard.example.com/check"}"#.to_owned(),
    ));
    let value = item_value(&item(&stored));
    let params = field(&value, "litellm_params");
    assert_eq!(text(field(params, "api_key")), "se****34");
    assert_eq!(
        text(field(params, "api_base")),
        "https://guard.example.com/check"
    );
}

#[test]
fn item_keeps_an_empty_params_map_empty() {
    let mut stored = row("g1", "guard", Some("active"), Some("team-a"));
    stored.litellm_params = Some(ColumnJson::Text("{}".to_owned()));
    let value = item_value(&item(&stored));
    assert_eq!(
        field(&value, "litellm_params"),
        &JsonData::Object(BTreeMap::new())
    );
    let mut absent = row("g2", "guard-two", Some("active"), Some("team-a"));
    absent.litellm_params = None;
    let absent_value = item_value(&item(&absent));
    assert_eq!(
        field(&absent_value, "litellm_params"),
        &JsonData::Object(BTreeMap::new())
    );
}

#[test]
fn item_defaults_a_missing_status_to_active() {
    let stored = row("g1", "guard", None, Some("team-a"));
    let value = item_value(&item(&stored));
    assert_eq!(text(field(&value, "status")), "active");
    let tagged = row("g2", "guard-two", Some("rejected"), Some("team-a"));
    assert_eq!(
        text(field(&item_value(&item(&tagged)), "status")),
        "rejected"
    );
}

#[test]
fn item_marks_team_guardrails() {
    let scoped = row("g1", "guard", Some("pending_review"), Some("team-a"));
    assert_eq!(
        field(&item_value(&item(&scoped)), "team_guardrail"),
        &JsonData::Bool(true)
    );
    let plain = row("g2", "guard-two", Some("active"), None);
    assert_eq!(
        field(&item_value(&item(&plain)), "team_guardrail"),
        &JsonData::Bool(false)
    );
}

#[test]
fn item_reads_the_submitter_from_guardrail_info() {
    let mut stored = row("g1", "guard", Some("pending_review"), Some("team-a"));
    stored.guardrail_info = Some(ColumnJson::Text(
        r#"{"submitted_by_user_id":"alice","submitted_by_email":"alice@example.com"}"#.to_owned(),
    ));
    let value = item_value(&item(&stored));
    assert_eq!(text(field(&value, "submitted_by_user_id")), "alice");
    assert_eq!(
        text(field(&value, "submitted_by_email")),
        "alice@example.com"
    );
    let mut keyed = row("g2", "guard-two", Some("pending_review"), Some("team-a"));
    keyed.guardrail_info = Some(ColumnJson::Text(
        r#"{"submitted_by_user_id":null}"#.to_owned(),
    ));
    let keyed_value = item_value(&item(&keyed));
    assert_eq!(field(&keyed_value, "submitted_by_email"), &JsonData::Null);
}

#[test]
fn item_gives_an_empty_info_object_when_the_column_is_null() {
    let mut stored = row("g1", "guard", Some("pending_review"), Some("team-a"));
    stored.guardrail_info = None;
    let value = item_value(&item(&stored));
    assert_eq!(
        field(&value, "guardrail_info"),
        &JsonData::Object(BTreeMap::new())
    );
    assert_eq!(field(&value, "submitted_by_user_id"), &JsonData::Null);
    assert_eq!(field(&value, "submitted_by_email"), &JsonData::Null);
}

#[test]
fn item_keeps_the_stored_timestamps() {
    let stored = row("g1", "guard", Some("pending_review"), Some("team-a"));
    let value = item_value(&item(&stored));
    let created = DateTime::parse_from_rfc3339(text(field(&value, "created_at")))
        .expect("a timestamp")
        .with_timezone(&Utc);
    assert_eq!(created, fixed_now());
    assert_eq!(field(&value, "reviewed_at"), &JsonData::Null);
    assert_ne!(field(&value, "submitted_at"), &JsonData::Null);
    assert_ne!(field(&value, "updated_at"), &JsonData::Null);
}
