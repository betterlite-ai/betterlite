use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;

use crate::auth_key_budgets::email::{
    alert_recipients, merge_budget_alert_email_configs, normalize_alert_emails, parse_email_list,
};
use crate::auth_key_budgets::key_budget::KeyBudgets;
use crate::auth_key_budgets::max_alert::virtual_key_max_budget_alert_check;
use crate::auth_key_budgets::money::Money;
use crate::auth_key_budgets::team_member::{
    TEAM_MEMBER_MAX_BUDGET_ALERT_EMAILS_KEY, team_member_max_budget_alert_check,
};

use super::{EventGroup, RecordingSink, dec, effective_budgets, emails, key, money, now};

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
    let merged = merge_budget_alert_email_configs(
        Some(&emails(r#"{"80":["a@x","b@y"]}"#)),
        Some(&emails(r#"{"80":["b@y","c@z"]}"#)),
    )
    .unwrap();
    assert_eq!(merged["80"], vec!["a@x", "b@y", "c@z"]);
}

#[test]
fn threshold_union() {
    let merged = merge_budget_alert_email_configs(
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
    assert_eq!(merge_budget_alert_email_configs(None, None), None);
    assert_eq!(
        merge_budget_alert_email_configs(Some(&BTreeMap::new()), Some(&BTreeMap::new())),
        None
    );
    assert_eq!(
        merge_budget_alert_email_configs(Some(&emails(r#"{"80":[]}"#)), None),
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
    let sink = RecordingSink::default();
    let auth = key(
        r#"{"token":"sk-map","spend":50,"max_budget":100,"metadata":{"max_budget_alert_emails":{"90":["high@x"],"50":["low@x"]}}}"#,
    );
    let budgets = effective_budgets(&auth, now()).unwrap();
    virtual_key_max_budget_alert_check(
        &auth,
        &budgets,
        None,
        None,
        Money::from_f64(0.8_f64),
        Some(&sink),
    );
    let alerts = sink.take();
    assert_eq!(alerts.len(), 1);
    let config = alerts[0].max_budget_alert_emails.as_ref().unwrap();
    assert_eq!(config["50"], vec!["low@x"]);
    assert_eq!(config["90"], vec!["high@x"]);

    let below = key(
        r#"{"token":"sk-map","spend":49,"max_budget":100,"metadata":{"max_budget_alert_emails":{"90":["high@x"],"50":["low@x"]}}}"#,
    );
    let budgets = effective_budgets(&below, now()).unwrap();
    virtual_key_max_budget_alert_check(
        &below,
        &budgets,
        None,
        None,
        Money::from_f64(0.8_f64),
        Some(&sink),
    );
    assert!(sink.take().is_empty());
}

#[test]
fn invalid_key_threshold_no_alert() {
    let sink = RecordingSink::default();
    let auth = key(
        r#"{"token":"sk-nodigit","spend":99,"max_budget":100,"metadata":{"max_budget_alert_emails":{"abc":["a@x"],"eighty":["b@x"]}}}"#,
    );
    let budgets = effective_budgets(&auth, now()).unwrap();
    virtual_key_max_budget_alert_check(
        &auth,
        &budgets,
        None,
        None,
        Money::from_f64(0.8_f64),
        Some(&sink),
    );
    assert!(sink.take().is_empty());
}

#[test]
fn key_threshold_outside_range_kept() {
    let sink = RecordingSink::default();
    let auth = key(
        r#"{"token":"sk-big","spend":160,"max_budget":100,"metadata":{"max_budget_alert_emails":{"150":["a@x"]}}}"#,
    );
    let budgets = effective_budgets(&auth, now()).unwrap();
    virtual_key_max_budget_alert_check(
        &auth,
        &budgets,
        None,
        None,
        Money::from_f64(0.8_f64),
        Some(&sink),
    );
    let alerts = sink.take();
    assert_eq!(alerts.len(), 1);
    assert_eq!(
        alerts[0].max_budget_alert_emails.as_ref().unwrap()["150"],
        vec!["a@x"]
    );

    let zero = key(
        r#"{"token":"sk-zero-pct","spend":1,"max_budget":100,"metadata":{"max_budget_alert_emails":{"0":["a@x"]}}}"#,
    );
    let budgets = effective_budgets(&zero, now()).unwrap();
    virtual_key_max_budget_alert_check(
        &zero,
        &budgets,
        None,
        None,
        Money::from_f64(0.8_f64),
        Some(&sink),
    );
    assert_eq!(sink.take().len(), 1);

    let huge = "9".repeat(400);
    let fields = format!(
        r#"{{"token":"sk-huge","spend":100,"max_budget":100,"metadata":{{"max_budget_alert_emails":{{"{huge}":["a@x"]}}}}}}"#
    );
    let auth = key(&fields);
    let budgets = effective_budgets(&auth, now()).unwrap();
    virtual_key_max_budget_alert_check(
        &auth,
        &budgets,
        None,
        None,
        Money::from_f64(0.8_f64),
        Some(&sink),
    );
    assert!(sink.take().is_empty());
}

#[test]
fn mapped_alert_at_or_over_max() {
    let sink = RecordingSink::default();
    for spend in [99_i64, 100_i64, 120_i64] {
        let fields = format!(
            r#"{{"token":"sk-at-max","spend":{spend},"max_budget":100,"metadata":{{"max_budget_alert_emails":{{"100":["a@x"]}}}}}}"#
        );
        let auth = key(&fields);
        let budgets = effective_budgets(&auth, now()).unwrap();
        virtual_key_max_budget_alert_check(
            &auth,
            &budgets,
            None,
            None,
            Money::from_f64(0.8_f64),
            Some(&sink),
        );
        let alerts = sink.take();
        assert_eq!(alerts.is_empty(), spend < 100_i64);
        if spend >= 100_i64 {
            assert_eq!(alerts[0].spend, dec(spend));
            assert!(alerts[0].max_budget_alert_emails.is_some());
        }
    }
}

#[test]
fn max_alert_nonfinite_budget() {
    let sink = RecordingSink::default();
    for (maximum, expected) in [
        (Money::Nan, true),
        (Money::PositiveInfinity, false),
        (Money::NegativeInfinity, true),
    ] {
        let auth = key(
            r#"{"token":"sk-nonfinite-alert","spend":100,"metadata":{"max_budget_alert_emails":{"100":["a@x"]}}}"#,
        );
        let budgets = KeyBudgets::from_auth(&auth).with_max_budget(Some(maximum));
        virtual_key_max_budget_alert_check(
            &auth,
            &budgets,
            None,
            None,
            Money::from_f64(0.8_f64),
            Some(&sink),
        );
        assert_eq!(!sink.take().is_empty(), expected);
    }
    let auth = key(
        r#"{"token":"sk-zero-pct","spend":5,"metadata":{"max_budget_alert_emails":{"0":["a@x"]}}}"#,
    );
    let budgets = KeyBudgets::from_auth(&auth).with_max_budget(Some(Money::NegativeInfinity));
    virtual_key_max_budget_alert_check(
        &auth,
        &budgets,
        None,
        None,
        Money::from_f64(0.8_f64),
        Some(&sink),
    );
    assert_eq!(sink.take().len(), 1);

    let legacy = key(r#"{"token":"sk-legacy-inf","spend":100}"#);
    for maximum in [Money::Nan, Money::PositiveInfinity, Money::NegativeInfinity] {
        let budgets = KeyBudgets::from_auth(&legacy).with_max_budget(Some(maximum));
        virtual_key_max_budget_alert_check(
            &legacy,
            &budgets,
            None,
            None,
            Money::from_f64(0.8_f64),
            Some(&sink),
        );
        assert!(sink.take().is_empty());
    }
}

#[test]
fn legacy_fraction_boundary() {
    let sink = RecordingSink::default();
    for (spend, expected) in [(79_i64, 0_usize), (80_i64, 1_usize), (99_i64, 1_usize)] {
        let fields = format!(r#"{{"token":"sk-legacy","spend":{spend},"max_budget":100}}"#);
        let auth = key(&fields);
        let budgets = effective_budgets(&auth, now()).unwrap();
        virtual_key_max_budget_alert_check(
            &auth,
            &budgets,
            None,
            None,
            Money::from_f64(0.8_f64),
            Some(&sink),
        );
        assert_eq!(sink.take().len(), expected);
    }
}

#[test]
fn legacy_max_excluded() {
    let sink = RecordingSink::default();
    for spend in [100_i64, 120_i64] {
        let fields = format!(r#"{{"token":"sk-legacy-max","spend":{spend},"max_budget":100}}"#);
        let auth = key(&fields);
        let budgets = effective_budgets(&auth, now()).unwrap();
        virtual_key_max_budget_alert_check(
            &auth,
            &budgets,
            None,
            None,
            Money::from_f64(0.8_f64),
            Some(&sink),
        );
        assert!(sink.take().is_empty());
    }
}

#[test]
fn member_invalid_map_silent() {
    let sink = RecordingSink::default();
    for config in ["\"not-a-map\"", "[\"50\"]", "null", "7"] {
        let metadata = emails(&format!(
            r#"{{"{TEAM_MEMBER_MAX_BUDGET_ALERT_EMAILS_KEY}":{config}}}"#
        ));
        team_member_max_budget_alert_check(
            "team-1",
            None,
            Some(&metadata),
            None,
            "user-1",
            None,
            dec(100),
            money(100),
            Some(&sink),
        );
    }
    assert!(sink.take().is_empty());
}

#[test]
fn member_threshold_bounds_length() {
    let sink = RecordingSink::default();
    let metadata = emails(&format!(
        r#"{{"{TEAM_MEMBER_MAX_BUDGET_ALERT_EMAILS_KEY}":{{"0":["zero@x"],"101":["over@x"],"1000":["long@x"],"50":["half@x"],"100":["max@x"]}}}}"#
    ));
    team_member_max_budget_alert_check(
        "team-1",
        None,
        Some(&metadata),
        None,
        "user-1",
        None,
        dec(50),
        money(100),
        Some(&sink),
    );
    let alerts = sink.take();
    assert_eq!(alerts.len(), 1);
    let config = alerts[0].max_budget_alert_emails.as_ref().unwrap();
    assert_eq!(config.len(), 2);
    assert_eq!(config["50"], vec!["half@x"]);
    assert_eq!(config["100"], vec!["max@x"]);

    let only_invalid = emails(&format!(
        r#"{{"{TEAM_MEMBER_MAX_BUDGET_ALERT_EMAILS_KEY}":{{"101":["over@x"]}}}}"#
    ));
    team_member_max_budget_alert_check(
        "team-1",
        None,
        Some(&only_invalid),
        None,
        "user-1",
        None,
        dec(100),
        money(100),
        Some(&sink),
    );
    assert!(sink.take().is_empty());
}

#[test]
fn member_recipients_coercion() {
    let sink = RecordingSink::default();
    let metadata = emails(&format!(
        r#"{{"{TEAM_MEMBER_MAX_BUDGET_ALERT_EMAILS_KEY}":{{"50":"a@x, b@y","70":[" a@x ","a@x","b@y",7]}}}}"#
    ));
    team_member_max_budget_alert_check(
        "team-1",
        None,
        Some(&metadata),
        None,
        "user-1",
        None,
        dec(60),
        money(100),
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
    let metadata = emails(&format!(
        r#"{{"{TEAM_MEMBER_MAX_BUDGET_ALERT_EMAILS_KEY}":{{"50":5}}}}"#
    ));
    team_member_max_budget_alert_check(
        "team-1",
        None,
        Some(&metadata),
        None,
        "user-1",
        None,
        dec(60),
        money(100),
        Some(&sink),
    );
    let alerts = sink.take();
    assert_eq!(alerts.len(), 1);
    assert!(alerts[0].max_budget_alert_emails.as_ref().unwrap()["50"].is_empty());
}

#[test]
fn member_no_global_merge() {
    let sink = RecordingSink::default();
    let global = emails(r#"{"50":["global@x"]}"#);
    let team_config = emails(r#"{"50":["team@x"]}"#);
    let key_auth = key(r#"{"token":"sk-team","spend":60,"max_budget":100}"#);
    let budgets = effective_budgets(&key_auth, now()).unwrap();
    virtual_key_max_budget_alert_check(
        &key_auth,
        &budgets,
        None,
        Some(&global),
        Money::from_f64(0.8_f64),
        Some(&sink),
    );
    let key_alerts = sink.take();
    assert_eq!(key_alerts.len(), 1);
    assert_eq!(
        key_alerts[0].max_budget_alert_emails.as_ref().unwrap()["50"],
        vec!["global@x"]
    );

    let merged = merge_budget_alert_email_configs(Some(&global), Some(&team_config)).unwrap();
    assert_eq!(merged["50"], vec!["global@x", "team@x"]);
    let metadata = emails(&format!(
        r#"{{"{TEAM_MEMBER_MAX_BUDGET_ALERT_EMAILS_KEY}":{{"50":["team@x"]}}}}"#
    ));
    team_member_max_budget_alert_check(
        "team-1",
        Some("team-a"),
        Some(&metadata),
        Some("org-1"),
        "user-1",
        Some("owner@x"),
        dec(60),
        money(100),
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
    assert_eq!(alert.token, None);
    assert_eq!(alert.key_alias, None);
    assert_eq!(alert.soft_budget, None);
}

#[test]
fn member_positive_spend_threshold() {
    let sink = RecordingSink::default();
    let metadata = emails(&format!(
        r#"{{"{TEAM_MEMBER_MAX_BUDGET_ALERT_EMAILS_KEY}":{{"50":["a@x"]}}}}"#
    ));
    for (spend, expected) in [
        (0_i64, false),
        (-5_i64, false),
        (49_i64, false),
        (50_i64, true),
        (150_i64, true),
    ] {
        team_member_max_budget_alert_check(
            "team-1",
            None,
            Some(&metadata),
            None,
            "user-1",
            None,
            dec(spend),
            money(100),
            Some(&sink),
        );
        assert_eq!(!sink.take().is_empty(), expected);
    }
}

fn json(text: &str) -> JsonData {
    serde_json::from_str(text).unwrap()
}
