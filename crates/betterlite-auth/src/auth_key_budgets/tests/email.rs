use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use rstest::rstest;

use crate::auth_key_budgets::email::{
    alert_recipients, is_valid_alert_threshold_pct, merge_alert_email_configs,
    normalize_alert_emails, parse_email_list,
};
use crate::auth_key_budgets::max_alert::max_budget_alert_check;
use crate::auth_key_budgets::money::Money;
use crate::auth_key_budgets::team_member::{
    TEAM_MEMBER_ALERT_KEY, TeamMemberAlert, team_member_alert_check,
};

use super::{
    EventGroup, RecordingSink, ScriptedReader, admission, dec, dec_text, effective_budgets, emails,
    key, money, now,
};

#[test]
fn list_and_csv_trim() {
    assert_eq!(
        parse_email_list(&json(r#"[" a@x ","", "b@y", 3, null]"#)),
        vec!["a@x", "b@y"]
    );
    assert_eq!(
        parse_email_list(&json(r#"" a@x , b@y ,, ""#)),
        vec!["a@x", "b@y"]
    );
    assert_eq!(parse_email_list(&JsonData::Null), Vec::<String>::new());
}

#[test]
fn nonlist_sequence_ignored() {
    assert!(parse_email_list(&json(r#"{"0":"a@x"}"#)).is_empty());
    assert!(parse_email_list(&json("true")).is_empty());
    assert!(parse_email_list(&json("7")).is_empty());
    assert_eq!(alert_recipients(&json("7")), None);
    assert_eq!(alert_recipients(&json(r#"{"a":"b"}"#)), None);
    assert_eq!(
        alert_recipients(&json(r#"["a@x"]"#)),
        Some(vec!["a@x".to_owned()])
    );
    assert_eq!(
        alert_recipients(&json(r#""a@x""#)),
        Some(vec!["a@x".to_owned()])
    );
}

#[test]
fn global_first_dedup() {
    let merged = merge_alert_email_configs(
        Some(&emails(r#"{"80":["a@x","b@y"]}"#)),
        Some(&emails(r#"{"80":["b@y","c@z"]}"#)),
    )
    .unwrap();
    assert_eq!(merged["80"], vec!["a@x", "b@y", "c@z"]);
}

#[test]
fn threshold_union() {
    let merged = merge_alert_email_configs(
        Some(&emails(r#"{"50":["g@x"]}"#)),
        Some(&emails(r#"{"90":["k@x"],"50":["k@x"]}"#)),
    )
    .unwrap();
    assert_eq!(merged.len(), 2);
    assert_eq!(merged["50"], vec!["g@x", "k@x"]);
    assert_eq!(merged["90"], vec!["k@x"]);
}

#[test]
fn empty_merge_none() {
    assert_eq!(merge_alert_email_configs(None, None), None);
    assert_eq!(
        merge_alert_email_configs(Some(&BTreeMap::new()), Some(&BTreeMap::new())),
        None
    );
    assert_eq!(
        merge_alert_email_configs(Some(&emails(r#"{"80":[]}"#)), None),
        Some(BTreeMap::from([("80".to_owned(), Vec::new())]))
    );
    assert_eq!(normalize_alert_emails(None), BTreeMap::new());
    assert_eq!(
        normalize_alert_emails(Some(&BTreeMap::new())),
        BTreeMap::new()
    );
}

#[test]
fn key_map_min_digit_threshold() {
    let reader = ScriptedReader::new(Vec::new());
    let sink = RecordingSink::default();
    let auth = key(
        r#"{"token":"sk-map","spend":50,"max_budget":100,"metadata":{"max_budget_alert_emails":{"90":["high@x"],"50":["low@x"]}}}"#,
    );
    let budgets = effective_budgets(&auth, now()).unwrap();
    max_budget_alert_check(&auth, &budgets, &admission(&reader, Some(&sink))).unwrap();
    let alerts = sink.take();
    assert_eq!(alerts.len(), 1);
    let config = alerts[0].max_budget_alert_emails.as_ref().unwrap();
    assert_eq!(config["50"], vec!["low@x"]);
    assert_eq!(config["90"], vec!["high@x"]);

    let below = key(
        r#"{"token":"sk-map","spend":49,"max_budget":100,"metadata":{"max_budget_alert_emails":{"90":["high@x"],"50":["low@x"]}}}"#,
    );
    let below_budgets = effective_budgets(&below, now()).unwrap();
    max_budget_alert_check(&below, &below_budgets, &admission(&reader, Some(&sink))).unwrap();
    assert!(sink.take().is_empty());
}

#[test]
fn invalid_key_threshold_no_alert() {
    let reader = ScriptedReader::new(Vec::new());
    let sink = RecordingSink::default();
    let auth = key(
        r#"{"token":"sk-nodigit","spend":99,"max_budget":100,"metadata":{"max_budget_alert_emails":{"abc":["a@x"],"eighty":["b@x"]}}}"#,
    );
    let budgets = effective_budgets(&auth, now()).unwrap();
    max_budget_alert_check(&auth, &budgets, &admission(&reader, Some(&sink))).unwrap();
    assert!(sink.take().is_empty());
}

#[test]
fn key_threshold_outside_range_kept() {
    let reader = ScriptedReader::new(Vec::new());
    let sink = RecordingSink::default();
    let auth = key(
        r#"{"token":"sk-big","spend":160,"max_budget":100,"metadata":{"max_budget_alert_emails":{"150":["a@x"]}}}"#,
    );
    let budgets = effective_budgets(&auth, now()).unwrap();
    max_budget_alert_check(&auth, &budgets, &admission(&reader, Some(&sink))).unwrap();
    let alerts = sink.take();
    assert_eq!(alerts.len(), 1);
    assert_eq!(
        alerts[0].max_budget_alert_emails.as_ref().unwrap()["150"],
        vec!["a@x"]
    );

    let zero = key(
        r#"{"token":"sk-zero-pct","spend":1,"max_budget":100,"metadata":{"max_budget_alert_emails":{"0":["a@x"]}}}"#,
    );
    let zero_budgets = effective_budgets(&zero, now()).unwrap();
    max_budget_alert_check(&zero, &zero_budgets, &admission(&reader, Some(&sink))).unwrap();
    assert_eq!(sink.take().len(), 1);

    let huge = "9".repeat(400);
    let fields = format!(
        r#"{{"token":"sk-huge","spend":100,"max_budget":100,"metadata":{{"max_budget_alert_emails":{{"{huge}":["a@x"]}}}}}}"#
    );
    let huge_auth = key(&fields);
    let huge_budgets = effective_budgets(&huge_auth, now()).unwrap();
    max_budget_alert_check(&huge_auth, &huge_budgets, &admission(&reader, Some(&sink))).unwrap();
    assert!(sink.take().is_empty());
}

#[rstest]
#[case::below(99_i64)]
#[case::at_max(100_i64)]
#[case::over_max(120_i64)]
fn mapped_alert_at_or_over_max(#[case] spend: i64) {
    let reader = ScriptedReader::new(Vec::new());
    let sink = RecordingSink::default();
    let fields = format!(
        r#"{{"token":"sk-at-max","spend":{spend},"max_budget":100,"metadata":{{"max_budget_alert_emails":{{"100":["a@x"]}}}}}}"#
    );
    let auth = key(&fields);
    let budgets = effective_budgets(&auth, now()).unwrap();
    max_budget_alert_check(&auth, &budgets, &admission(&reader, Some(&sink))).unwrap();
    let alerts = sink.take();
    assert_eq!(alerts.is_empty(), spend < 100_i64);
    if spend >= 100_i64 {
        assert_eq!(alerts[0].spend, dec(spend));
        assert!(alerts[0].max_budget_alert_emails.is_some());
    }
}

#[rstest]
#[case::nan(Money::Nan, 1_usize)]
#[case::positive_infinity(Money::PositiveInfinity, 0_usize)]
#[case::negative_infinity(Money::NegativeInfinity, 1_usize)]
fn max_alert_nonfinite_budget(#[case] maximum: Money, #[case] mapped_alerts: usize) {
    let reader = ScriptedReader::new(Vec::new());
    let sink = RecordingSink::default();
    let auth = key(
        r#"{"token":"sk-nonfinite-alert","spend":100,"metadata":{"max_budget_alert_emails":{"100":["a@x"]}}}"#,
    );
    let mut budgets = effective_budgets(&auth, now()).unwrap();
    budgets.max_budget = Some(maximum);
    max_budget_alert_check(&auth, &budgets, &admission(&reader, Some(&sink))).unwrap();
    assert_eq!(sink.take().len(), mapped_alerts);

    let zero_pct = key(
        r#"{"token":"sk-zero-pct","spend":5,"metadata":{"max_budget_alert_emails":{"0":["a@x"]}}}"#,
    );
    let mut zero_budgets = effective_budgets(&zero_pct, now()).unwrap();
    zero_budgets.max_budget = Some(Money::NegativeInfinity);
    max_budget_alert_check(&zero_pct, &zero_budgets, &admission(&reader, Some(&sink))).unwrap();
    assert_eq!(sink.take().len(), 1);

    let legacy = key(r#"{"token":"sk-legacy-inf","spend":100}"#);
    let mut legacy_budgets = effective_budgets(&legacy, now()).unwrap();
    legacy_budgets.max_budget = Some(maximum);
    max_budget_alert_check(&legacy, &legacy_budgets, &admission(&reader, Some(&sink))).unwrap();
    assert!(sink.take().is_empty());
}

#[rstest]
#[case::below(79_i64, 0_usize)]
#[case::at_threshold(80_i64, 1_usize)]
#[case::below_max(99_i64, 1_usize)]
fn legacy_fraction_boundary(#[case] spend: i64, #[case] expected: usize) {
    let reader = ScriptedReader::new(Vec::new());
    let sink = RecordingSink::default();
    let fields = format!(r#"{{"token":"sk-legacy","spend":{spend},"max_budget":100}}"#);
    let auth = key(&fields);
    let budgets = effective_budgets(&auth, now()).unwrap();
    max_budget_alert_check(&auth, &budgets, &admission(&reader, Some(&sink))).unwrap();
    assert_eq!(sink.take().len(), expected);
}

#[rstest]
#[case::at_max(100_i64)]
#[case::over_max(120_i64)]
fn legacy_max_excluded(#[case] spend: i64) {
    let reader = ScriptedReader::new(Vec::new());
    let sink = RecordingSink::default();
    let fields = format!(r#"{{"token":"sk-legacy-max","spend":{spend},"max_budget":100}}"#);
    let auth = key(&fields);
    let budgets = effective_budgets(&auth, now()).unwrap();
    max_budget_alert_check(&auth, &budgets, &admission(&reader, Some(&sink))).unwrap();
    assert!(sink.take().is_empty());
}

#[test]
fn mapped_threshold_exact_decimal() {
    let reader = ScriptedReader::new(Vec::new());
    let sink = RecordingSink::default();
    let auth = key(
        r#"{"token":"sk-tenth","spend":0.08,"max_budget":0.1,"metadata":{"max_budget_alert_emails":{"80":["a@x"]}}}"#,
    );
    let budgets = effective_budgets(&auth, now()).unwrap();
    max_budget_alert_check(&auth, &budgets, &admission(&reader, Some(&sink))).unwrap();
    assert_eq!(
        sink.take().len(),
        1,
        "0.08 is exactly 80% of 0.1, where LiteLLM's 0.1 * 0.8 falls short"
    );

    let legacy = key(r#"{"token":"sk-tenth-legacy","spend":0.08,"max_budget":0.1}"#);
    let legacy_budgets = effective_budgets(&legacy, now()).unwrap();
    max_budget_alert_check(&legacy, &legacy_budgets, &admission(&reader, Some(&sink))).unwrap();
    assert_eq!(
        sink.take().len(),
        1,
        "the legacy fraction scales the same maximum by the same exact 0.8"
    );

    let metadata = emails(r#"{"team_member_max_budget_alert_emails":{"80":["a@x"]}}"#);
    team_member_alert_check(
        &TeamMemberAlert {
            team_id: "team-1",
            team_alias: None,
            team_metadata: Some(&metadata),
            organization_id: None,
            user_id: "user-1",
            user_email: None,
            spend: dec_text("0.08"),
            max_budget: Money::Finite(dec_text("0.1")),
        },
        Some(&sink),
    );
    assert_eq!(
        sink.take().len(),
        1,
        "the team threshold scales the same maximum by the same exact 0.8"
    );
}

#[test]
fn non_ascii_digit_thresholds_dropped() {
    let reader = ScriptedReader::new(Vec::new());
    let sink = RecordingSink::default();
    // U+0665 U+0660 spell the Arabic-Indic 50, which Python's str.isdigit reads
    let threshold = String::from_utf8(vec![0xD9_u8, 0xA5, 0xD9, 0xA0]).unwrap();
    let fields = format!(
        r#"{{"token":"sk-unicode","spend":99,"max_budget":100,"metadata":{{"max_budget_alert_emails":{{"{threshold}":["a@x"]}}}}}}"#
    );
    let auth = key(&fields);
    let budgets = effective_budgets(&auth, now()).unwrap();
    max_budget_alert_check(&auth, &budgets, &admission(&reader, Some(&sink))).unwrap();
    assert!(
        sink.take().is_empty(),
        "Arabic-Indic digits are not the ASCII digits a threshold key uses"
    );

    let metadata = emails(&format!(
        r#"{{"{TEAM_MEMBER_ALERT_KEY}":{{"{threshold}":["a@x"]}}}}"#
    ));
    team_member_alert_check(
        &TeamMemberAlert {
            team_id: "team-1",
            team_alias: None,
            team_metadata: Some(&metadata),
            organization_id: None,
            user_id: "user-1",
            user_email: None,
            spend: dec(100),
            max_budget: money(100),
        },
        Some(&sink),
    );
    assert!(sink.take().is_empty());
}

#[rstest]
#[case::string("\"not-a-map\"")]
#[case::array("[\"50\"]")]
#[case::null("null")]
#[case::number("7")]
fn member_invalid_map_silent(#[case] config: &str) {
    let sink = RecordingSink::default();
    let metadata = emails(&format!(r#"{{"{TEAM_MEMBER_ALERT_KEY}":{config}}}"#));
    team_member_alert_check(
        &TeamMemberAlert {
            team_id: "team-1",
            team_alias: None,
            team_metadata: Some(&metadata),
            organization_id: None,
            user_id: "user-1",
            user_email: None,
            spend: dec(100),
            max_budget: money(100),
        },
        Some(&sink),
    );
    assert!(sink.take().is_empty());
}

#[rstest]
#[case::single_digit("1", true)]
#[case::leading_zero("050", true)]
#[case::hundred("100", true)]
#[case::four_digit_one("0001", false)]
#[case::four_digit_hundred("0100", false)]
#[case::empty("", false)]
#[case::over_hundred("101", false)]
#[case::non_digit("1a", false)]
fn threshold_pct_requires_three_ascii_digits(#[case] pct: &str, #[case] valid: bool) {
    assert_eq!(is_valid_alert_threshold_pct(pct), valid, "{pct}");
}

#[test]
fn member_threshold_bounds_length() {
    let sink = RecordingSink::default();
    let metadata = emails(&format!(
        r#"{{"{TEAM_MEMBER_ALERT_KEY}":{{"0":["zero@x"],"101":["over@x"],"1000":["long@x"],"0001":["leading@x"],"50":["half@x"],"100":["max@x"]}}}}"#
    ));
    team_member_alert_check(
        &TeamMemberAlert {
            team_id: "team-1",
            team_alias: None,
            team_metadata: Some(&metadata),
            organization_id: None,
            user_id: "user-1",
            user_email: None,
            spend: dec(50),
            max_budget: money(100),
        },
        Some(&sink),
    );
    let alerts = sink.take();
    assert_eq!(alerts.len(), 1);
    let config = alerts[0].max_budget_alert_emails.as_ref().unwrap();
    assert_eq!(config.len(), 2);
    assert_eq!(config["50"], vec!["half@x"]);
    assert_eq!(config["100"], vec!["max@x"]);

    let only_invalid = emails(&format!(
        r#"{{"{TEAM_MEMBER_ALERT_KEY}":{{"101":["over@x"],"0001":["leading@x"]}}}}"#
    ));
    team_member_alert_check(
        &TeamMemberAlert {
            team_id: "team-1",
            team_alias: None,
            team_metadata: Some(&only_invalid),
            organization_id: None,
            user_id: "user-1",
            user_email: None,
            spend: dec(100),
            max_budget: money(100),
        },
        Some(&sink),
    );
    assert!(sink.take().is_empty());
}

#[test]
fn member_recipients_coercion() {
    let sink = RecordingSink::default();
    let metadata = emails(&format!(
        r#"{{"{TEAM_MEMBER_ALERT_KEY}":{{"50":"a@x, b@y","70":[" a@x ","a@x","b@y",7]}}}}"#
    ));
    team_member_alert_check(
        &TeamMemberAlert {
            team_id: "team-1",
            team_alias: None,
            team_metadata: Some(&metadata),
            organization_id: None,
            user_id: "user-1",
            user_email: None,
            spend: dec(60),
            max_budget: money(100),
        },
        Some(&sink),
    );
    let alerts = sink.take();
    assert_eq!(alerts.len(), 1);
    let config = alerts[0].max_budget_alert_emails.as_ref().unwrap();
    assert_eq!(config["50"], vec!["a@x", "b@y"]);
    assert_eq!(config["70"], vec!["a@x", "b@y"]);
}

#[test]
fn member_unsupported_recipients_keep_threshold() {
    let sink = RecordingSink::default();
    let metadata = emails(&format!(r#"{{"{TEAM_MEMBER_ALERT_KEY}":{{"50":5}}}}"#));
    team_member_alert_check(
        &TeamMemberAlert {
            team_id: "team-1",
            team_alias: None,
            team_metadata: Some(&metadata),
            organization_id: None,
            user_id: "user-1",
            user_email: None,
            spend: dec(60),
            max_budget: money(100),
        },
        Some(&sink),
    );
    let alerts = sink.take();
    assert_eq!(alerts.len(), 1);
    assert!(alerts[0].max_budget_alert_emails.as_ref().unwrap()["50"].is_empty());
}

#[test]
fn key_alert_uses_global_emails() {
    let reader = ScriptedReader::new(Vec::new());
    let sink = RecordingSink::default();
    let global = emails(r#"{"50":["global@x"]}"#);
    let key_auth = key(r#"{"token":"sk-team","spend":60,"max_budget":100}"#);
    let budgets = effective_budgets(&key_auth, now()).unwrap();
    let mut inputs = admission(&reader, Some(&sink));
    inputs.global_alert_emails = Some(&global);
    max_budget_alert_check(&key_auth, &budgets, &inputs).unwrap();
    let key_alerts = sink.take();
    assert_eq!(key_alerts.len(), 1);
    assert_eq!(
        key_alerts[0].max_budget_alert_emails.as_ref().unwrap()["50"],
        vec!["global@x"]
    );
}

#[test]
fn member_no_global_merge() {
    let sink = RecordingSink::default();
    let global = emails(r#"{"50":["global@x"]}"#);
    let team_config = emails(r#"{"50":["team@x"]}"#);
    let merged = merge_alert_email_configs(Some(&global), Some(&team_config)).unwrap();
    assert_eq!(merged["50"], vec!["global@x", "team@x"]);
    let metadata = emails(&format!(
        r#"{{"{TEAM_MEMBER_ALERT_KEY}":{{"50":["team@x"]}}}}"#
    ));
    team_member_alert_check(
        &TeamMemberAlert {
            team_id: "team-1",
            team_alias: Some("team-a"),
            team_metadata: Some(&metadata),
            organization_id: Some("org-1"),
            user_id: "user-1",
            user_email: Some("owner@x"),
            spend: dec(60),
            max_budget: money(100),
        },
        Some(&sink),
    );
    let member_alerts = sink.take();
    assert_eq!(member_alerts.len(), 1);
    let alert = &member_alerts[0];
    assert_eq!(
        alert.max_budget_alert_emails.as_ref().unwrap()["50"],
        vec!["team@x"]
    );
    assert_eq!(alert.event_group, EventGroup::TeamMember);
    assert_eq!(alert.team_alias.as_deref(), Some("team-a"));
    assert_eq!(alert.organization_id.as_deref(), Some("org-1"));
    assert_eq!(alert.user_email.as_deref(), Some("owner@x"));
    assert!(alert.token.is_none());
    assert_eq!(alert.key_alias, None);
    assert_eq!(alert.soft_budget, None);
}

#[rstest]
#[case::zero(0_i64, false)]
#[case::negative(-5_i64, false)]
#[case::below(49_i64, false)]
#[case::at(50_i64, true)]
#[case::over(150_i64, true)]
fn member_positive_spend_threshold(#[case] spend: i64, #[case] expected: bool) {
    let sink = RecordingSink::default();
    let metadata = emails(&format!(
        r#"{{"{TEAM_MEMBER_ALERT_KEY}":{{"50":["a@x"]}}}}"#
    ));
    team_member_alert_check(
        &TeamMemberAlert {
            team_id: "team-1",
            team_alias: None,
            team_metadata: Some(&metadata),
            organization_id: None,
            user_id: "user-1",
            user_email: None,
            spend: dec(spend),
            max_budget: money(100),
        },
        Some(&sink),
    );
    assert_eq!(!sink.take().is_empty(), expected);
}

#[test]
fn member_zero_spend_zero_budget_silent() {
    let sink = RecordingSink::default();
    let metadata = emails(&format!(
        r#"{{"{TEAM_MEMBER_ALERT_KEY}":{{"50":["a@x"]}}}}"#
    ));
    team_member_alert_check(
        &TeamMemberAlert {
            team_id: "team-1",
            team_alias: None,
            team_metadata: Some(&metadata),
            organization_id: None,
            user_id: "user-1",
            user_email: None,
            spend: dec(0),
            max_budget: money(0),
        },
        Some(&sink),
    );
    assert!(
        sink.take().is_empty(),
        "a zero spend never alerts, even when a zero maximum meets the threshold"
    );
}

fn json(text: &str) -> JsonData {
    serde_json::from_str(text).unwrap()
}
