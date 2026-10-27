#![cfg(test)]

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::str::FromStr as _;

use indexmap::IndexMap;
use rstest::rstest;
use rust_decimal::Decimal;

use crate::records::{ROIEstimate, ROIEstimateStatus, ROIPullRecord, ROIReport, ROISpendRecord};
use crate::summary::error::SummaryError;
use crate::summary::identity::{MatchMethod, PullIdentity, match_identity};
use crate::summary::identity_map::{ROIIdentityMapResponse, ROIIdentityMapUpdate};
use crate::summary::models::{ROIReportResponse, ROISummary};
use crate::summary::summarize;

const DEV: &str = "dev@example.com";

fn dec(value: &str) -> Decimal {
    Decimal::from_str(value).unwrap()
}

fn spend(user_id: &str, email: &str, amount: &str) -> ROISpendRecord {
    ROISpendRecord {
        date: "2026-01-02".to_owned(),
        user_id: user_id.to_owned(),
        email: email.to_owned(),
        spend: dec(amount),
        requests: 1,
    }
}

fn dated(date: &str, mut record: ROISpendRecord) -> ROISpendRecord {
    record.date = date.to_owned();
    record
}

fn pull(login: &str, emails: &[&str], hours: Option<f64>) -> ROIPullRecord {
    ROIPullRecord {
        repo: "o/r".to_owned(),
        number: 1,
        title: "t".to_owned(),
        url: "https://github.com/o/r/pull/1".to_owned(),
        login: login.to_owned(),
        emails: emails.iter().map(|email| (*email).to_owned()).collect(),
        profile_email: emails
            .first()
            .map_or_else(String::new, |email| (*email).to_owned()),
        commit_emails: Some(vec!["raw@example.com".to_owned()]),
        merged_at: "2026-01-02T10:00:00Z".to_owned(),
        head_sha: "abc".to_owned(),
        additions: 1,
        deletions: 2,
        changed_files: 3,
        commit_count: 4,
        incomplete_metadata: false,
        estimate: ROIEstimate {
            status: ROIEstimateStatus::Estimated,
            hours,
            reasoning: "r".to_owned(),
            model: None,
            evidence_source: None,
            effort_basis: None,
            cached: None,
        },
        cache_key: None,
    }
}

fn with_status(mut record: ROIPullRecord, status: ROIEstimateStatus) -> ROIPullRecord {
    record.estimate.status = status;
    record
}

fn with_profile(mut record: ROIPullRecord, profile_email: &str) -> ROIPullRecord {
    record.profile_email = profile_email.to_owned();
    record
}

fn merged(number: i64, merged_at: &str, mut record: ROIPullRecord) -> ROIPullRecord {
    record.number = number;
    record.merged_at = merged_at.to_owned();
    record
}

fn report(spend: Vec<ROISpendRecord>, pulls: Vec<ROIPullRecord>) -> ROIReport {
    ROIReport {
        mode: "live".to_owned(),
        start: "2026-01-01".to_owned(),
        end: "2026-01-08".to_owned(),
        synced_at: "2026-01-08T00:00:00Z".to_owned(),
        repos: vec!["o/r".to_owned()],
        estimator_model: "model".to_owned(),
        estimator_prompt: "prompt".to_owned(),
        effort_basis: "basis".to_owned(),
        spend,
        pulls,
        settings_fingerprint: "f".to_owned(),
        warnings: None,
        unavailable_repos: None,
        id: None,
    }
}

fn scoped(unavailable: Vec<&str>, mut source: ROIReport) -> ROIReport {
    source.unavailable_repos = Some(unavailable.into_iter().map(str::to_owned).collect());
    source
}

fn one_person(amount: &str, hours: Option<f64>) -> ROIReport {
    report(
        vec![spend("user-1", DEV, amount)],
        vec![pull("dev", &[DEV], hours)],
    )
}

fn mappings(pairs: &[(&str, &str)]) -> IndexMap<String, String> {
    pairs
        .iter()
        .map(|&(login, email)| (login.to_owned(), email.to_owned()))
        .collect()
}

fn summary(source: &ROIReport) -> ROISummary {
    summarize(source, &IndexMap::new()).unwrap()
}

fn team() -> ROIReport {
    report(
        vec![
            dated("2026-01-01", spend("user-1", DEV, "3")),
            dated("2026-01-02", spend("user-1", DEV, "1")),
            dated("2026-01-02", spend("user-2", "ops@example.com", "5")),
        ],
        vec![
            merged(
                1,
                "2026-01-02T09:00:00Z",
                pull("dev", &[DEV], Some(2.0_f64)),
            ),
            merged(2, "2026-01-04T09:00:00Z", pull("guest", &[], Some(1.0_f64))),
            merged(
                3,
                "2026-01-04T10:00:00Z",
                with_status(pull("guest", &[], None), ROIEstimateStatus::NeedsReview),
            ),
        ],
    )
}

#[rstest]
#[case::manual_override_wins(
    &[("dev", "Manual@Example.com")], "DEV", &[DEV], ("manual@example.com", MatchMethod::Manual)
)]
#[case::manual_login_is_casefolded(
    &[("strasse", "m@example.com")], "Stra\u{df}e", &[], ("m@example.com", MatchMethod::Manual)
)]
#[case::manual_ignores_observed_emails(
    &[("dev", "elsewhere@example.com")], "dev", &[DEV],
    ("elsewhere@example.com", MatchMethod::Manual)
)]
#[case::empty_mapping_falls_through(
    &[("dev", "")], "dev", &[DEV], (DEV, MatchMethod::ProfileEmail)
)]
#[case::profile_email(&[], "dev", &["Dev@Example.com"], (DEV, MatchMethod::ProfileEmail))]
#[case::commit_email(&[], "dev", &["home@example.com", DEV], (DEV, MatchMethod::CommitEmail))]
#[case::duplicate_candidates_collapse(
    &[], "dev", &["home@example.com", DEV, "DEV@example.com"], (DEV, MatchMethod::CommitEmail)
)]
#[case::ambiguous_emails(
    &[], "dev", &[DEV, "second@example.com"], ("", MatchMethod::AmbiguousEmails)
)]
#[case::no_candidates(&[], "dev", &[], ("", MatchMethod::EmailUnavailable))]
#[case::only_invalid_candidates(
    &[], "dev", &["1+dev@users.noreply.github.com", "not-an-email"],
    ("", MatchMethod::EmailUnavailable)
)]
#[case::no_gateway_match(&[], "dev", &["home@example.com"], ("", MatchMethod::NoGatewayMatch))]
fn identity_matching(
    #[case] pairs: &[(&str, &str)],
    #[case] login: &str,
    #[case] emails: &[&str],
    #[case] expected: (&str, MatchMethod),
) {
    let observed = BTreeSet::from([DEV.to_owned(), "second@example.com".to_owned()]);
    let (email, method) = expected;
    assert_eq!(
        match_identity(&pull(login, emails, None), &observed, &mappings(pairs)),
        PullIdentity {
            email: email.to_owned(),
            method
        }
    );
}

#[rstest]
#[case::spend_and_pull_collapse_by_email(
    report(vec![spend("user-1", DEV, "4")], vec![pull("dev", &[DEV], Some(2.0_f64))]),
    &[DEV]
)]
#[case::spend_email_is_normalized(report(vec![spend("user-1", " Dev@Example.COM ", "1")], vec![]), &[DEV])]
#[case::empty_email_keys_by_user(report(vec![spend("user-1", "", "1")], vec![]), &["gateway:user-1"])]
#[case::invalid_email_keys_by_user(
    report(vec![spend("user-1", "dev@localhost", "1")], vec![]), &["gateway:user-1"]
)]
#[case::noreply_email_keys_by_user(
    report(vec![spend("user-1", "1+dev@users.noreply.github.com", "1")], vec![]),
    &["gateway:user-1"]
)]
#[case::unmatched_pull_keys_by_casefolded_login(
    report(vec![], vec![pull("Stra\u{df}e", &[], None)]), &["github:strasse"]
)]
#[case::unmatched_rows_stay_apart(
    report(vec![spend("user-1", "", "4")], vec![pull("Dev", &[], Some(2.0_f64))]),
    &["github:dev", "gateway:user-1"]
)]
fn person_key(#[case] source: ROIReport, #[case] ids: &[&str]) {
    let result = summary(&source);
    let found: Vec<&str> = result
        .people
        .iter()
        .map(|person| person.id.as_str())
        .collect();
    assert_eq!(found, ids);
}

#[rstest]
#[case::matched_profile_email(&[], one_person("1", None), (DEV, MatchMethod::ProfileEmail, true))]
#[case::matched_commit_email(
    &[], report(vec![spend("user-1", DEV, "1")], vec![with_profile(pull("dev", &[DEV], None), "")]),
    (DEV, MatchMethod::CommitEmail, true)
)]
#[case::manual_and_observed(&[("dev", DEV)], one_person("1", None), (DEV, MatchMethod::Manual, true))]
#[case::manual_but_unobserved(
    &[("dev", "new@example.com")], one_person("1", None),
    ("new@example.com", MatchMethod::Manual, false)
)]
#[case::no_gateway_match(
    &[], report(vec![], vec![pull("dev", &[DEV], None)]), ("", MatchMethod::NoGatewayMatch, false)
)]
#[case::ambiguous_emails(
    &[], report(
        vec![spend("user-1", DEV, "1"), spend("user-2", "second@example.com", "1")],
        vec![pull("dev", &[DEV, "second@example.com"], None)],
    ),
    ("", MatchMethod::AmbiguousEmails, false)
)]
fn pull_summary(
    #[case] pairs: &[(&str, &str)],
    #[case] source: ROIReport,
    #[case] expected: (&str, MatchMethod, bool),
) {
    let result = summarize(&source, &mappings(pairs)).unwrap();
    let found = result.pulls.first().unwrap();
    assert_eq!(
        (found.email.as_str(), found.match_method, found.matched),
        expected
    );
}

#[rstest]
#[case::aggregates_spend_hours_and_counts(
    &[],
    report(
        vec![spend("user-1", DEV, "5"), spend("user-1", "DEV@example.com", "2.5")],
        vec![
            pull("dev", &[DEV], Some(2.0_f64)),
            pull("dev", &[DEV], None),
            with_status(pull("dev", &[DEV], Some(9.0_f64)), ROIEstimateStatus::NeedsReview),
            with_status(pull("dev", &[DEV], None), ROIEstimateStatus::Error),
        ],
    ),
    DEV,
    r#"{"id":"dev@example.com","email":"dev@example.com","logins":["dev"],"spend":7.5,"hours":2,"prs":4,"estimated_prs":2,"pending_prs":2,"match_methods":["profile email"],"eligible":false,"cost_per_hour":null}"#
)]
#[case::keeps_first_logins_and_methods_in_order(
    &[],
    report(
        vec![spend("user-1", DEV, "1")],
        vec![
            with_profile(pull("Bob", &[DEV], Some(1.0_f64)), "home@example.com"),
            pull("alice", &[DEV], Some(1.0_f64)),
            with_profile(pull("Bob", &[DEV], Some(1.0_f64)), "home@example.com"),
        ],
    ),
    DEV,
    r#"{"id":"dev@example.com","email":"dev@example.com","logins":["Bob","alice"],"spend":1,"hours":3,"prs":3,"estimated_prs":3,"pending_prs":0,"match_methods":["commit email","profile email"],"eligible":true,"cost_per_hour":0.3333333333333333333333333333}"#
)]
#[case::no_spend_is_not_eligible(
    &[("dev", DEV)],
    report(vec![], vec![pull("dev", &[], Some(1.0_f64))]),
    DEV,
    r#"{"id":"dev@example.com","email":"dev@example.com","logins":["dev"],"spend":null,"hours":1,"prs":1,"estimated_prs":1,"pending_prs":0,"match_methods":["manual"],"eligible":false,"cost_per_hour":null}"#
)]
#[case::no_estimated_pull_is_not_eligible(
    &[],
    report(
        vec![spend("user-1", DEV, "1")],
        vec![with_status(pull("dev", &[DEV], None), ROIEstimateStatus::Error)],
    ),
    DEV,
    r#"{"id":"dev@example.com","email":"dev@example.com","logins":["dev"],"spend":1,"hours":0,"prs":1,"estimated_prs":0,"pending_prs":1,"match_methods":["profile email"],"eligible":false,"cost_per_hour":null}"#
)]
#[case::pending_pull_is_not_eligible(
    &[],
    report(
        vec![spend("user-1", DEV, "1")],
        vec![
            pull("dev", &[DEV], Some(1.0_f64)),
            with_status(pull("dev", &[DEV], None), ROIEstimateStatus::NeedsReview),
        ],
    ),
    DEV,
    r#"{"id":"dev@example.com","email":"dev@example.com","logins":["dev"],"spend":1,"hours":1,"prs":2,"estimated_prs":1,"pending_prs":1,"match_methods":["profile email"],"eligible":false,"cost_per_hour":null}"#
)]
#[case::incomplete_scope_has_no_cost_per_hour(
    &[],
    scoped(vec!["o/gone"], one_person("6", Some(3.0_f64))),
    DEV,
    r#"{"id":"dev@example.com","email":"dev@example.com","logins":["dev"],"spend":6,"hours":3,"prs":1,"estimated_prs":1,"pending_prs":0,"match_methods":["profile email"],"eligible":true,"cost_per_hour":null}"#
)]
#[case::empty_unavailable_list_is_complete(
    &[],
    scoped(vec![], one_person("6", Some(3.0_f64))),
    DEV,
    r#"{"id":"dev@example.com","email":"dev@example.com","logins":["dev"],"spend":6,"hours":3,"prs":1,"estimated_prs":1,"pending_prs":0,"match_methods":["profile email"],"eligible":true,"cost_per_hour":2}"#
)]
#[case::zero_hours_has_no_cost_per_hour(
    &[],
    one_person("6", None),
    DEV,
    r#"{"id":"dev@example.com","email":"dev@example.com","logins":["dev"],"spend":6,"hours":0,"prs":1,"estimated_prs":1,"pending_prs":0,"match_methods":["profile email"],"eligible":true,"cost_per_hour":null}"#
)]
#[case::first_spend_email_wins_even_when_invalid(
    &[],
    report(vec![spend("user-1", "dev@localhost", "1"), spend("user-1", "", "1")], vec![]),
    "gateway:user-1",
    r#"{"id":"gateway:user-1","email":"","logins":[],"spend":2,"hours":0,"prs":0,"estimated_prs":0,"pending_prs":0,"match_methods":[],"eligible":false,"cost_per_hour":null}"#
)]
fn person_summary(
    #[case] pairs: &[(&str, &str)],
    #[case] source: ROIReport,
    #[case] id: &str,
    #[case] expected: &str,
) {
    let result = summarize(&source, &mappings(pairs)).unwrap();
    let found = result.people.iter().find(|person| person.id == id).unwrap();
    assert_eq!(serde_json::to_string(found).unwrap(), expected);
}

#[rstest]
#[case::complete_scope(one_person("10", Some(4.0_f64)), (Some("2.5"), Some("0.4")))]
#[case::unavailable_repos(scoped(vec!["o/gone"], one_person("10", Some(4.0_f64))), (None, None))]
#[case::zero_output_hours(one_person("10", None), (None, Some("0")))]
#[case::zero_matched_spend(one_person("0", Some(4.0_f64)), (Some("0"), None))]
#[case::no_cohort(report(vec![spend("user-1", DEV, "10")], vec![]), (None, None))]
fn ratio_metrics(#[case] source: ROIReport, #[case] expected: (Option<&str>, Option<&str>)) {
    let metrics = summary(&source).metrics;
    let (cost_per_hour, hours_per_dollar) = expected;
    assert_eq!(
        (metrics.cost_per_hour, metrics.hours_per_dollar),
        (cost_per_hour.map(dec), hours_per_dollar.map(dec))
    );
}

#[rstest]
#[case::people_by_hours_then_id(
    report(
        vec![spend("user-9", "", "100")],
        vec![
            merged(1, "2026-01-02T00:00:00Z", pull("carol", &[], Some(1.0_f64))),
            merged(2, "2026-01-02T00:00:00Z", pull("bob", &[], Some(3.0_f64))),
            merged(3, "2026-01-02T00:00:00Z", pull("alice", &[], Some(3.0_f64))),
        ],
    ),
    &["github:alice", "github:bob", "github:carol", "gateway:user-9"],
    &[1, 2, 3]
)]
#[case::pulls_newest_first_keeping_ties_in_order(
    report(
        vec![],
        vec![
            merged(1, "2026-01-03T00:00:00Z", pull("a", &[], None)),
            merged(2, "2026-01-05T00:00:00Z", pull("a", &[], None)),
            merged(3, "2026-01-03T00:00:00Z", pull("a", &[], None)),
        ],
    ),
    &["github:a"],
    &[2, 1, 3]
)]
fn summary_sorting(#[case] source: ROIReport, #[case] people: &[&str], #[case] pulls: &[i64]) {
    let result = summary(&source);
    let ids: Vec<&str> = result
        .people
        .iter()
        .map(|person| person.id.as_str())
        .collect();
    let numbers: Vec<i64> = result.pulls.iter().map(|found| found.number).collect();
    assert_eq!((ids.as_slice(), numbers.as_slice()), (people, pulls));
}

#[test]
fn trend_counts_only_the_eligible_cohort() {
    let trend = serde_json::to_string(&summary(&team()).trend).unwrap();
    assert_eq!(
        trend,
        r#"[{"date":"2026-01-01","spend":3,"hours":0,"prs":0},{"date":"2026-01-02","spend":1,"hours":2,"prs":1},{"date":"2026-01-04","spend":0,"hours":0,"prs":0}]"#
    );
}

#[test]
fn metrics_split_the_cohort_from_totals() {
    let metrics = serde_json::to_string(&summary(&team()).metrics).unwrap();
    assert_eq!(
        metrics,
        r#"{"matched_spend":4,"output_hours":2,"total_spend":9,"total_output_hours":3,"excluded_spend":5,"cost_per_hour":2,"hours_per_dollar":0.5,"merged_prs":3,"estimated_prs":2,"matched_prs":1,"cohort_people":1,"people_with_prs":2,"pending_prs":1}"#
    );
}

#[test]
fn excluded_spend_never_goes_negative() {
    let source = report(
        vec![spend("user-1", DEV, "4"), spend("user-2", "", "-2")],
        vec![pull("dev", &[DEV], Some(1.0_f64))],
    );
    assert_eq!(summary(&source).metrics.excluded_spend, Decimal::ZERO);
}

#[test]
fn copies_the_report_header() {
    let mut source = report(vec![], vec![]);
    source.id = Some("report-1".to_owned());
    source.warnings = Some(vec!["partial".to_owned()]);
    let result = summary(&source);
    assert_eq!(
        (result.id.as_deref(), result.warnings.as_slice()),
        (Some("report-1"), ["partial".to_owned()].as_slice())
    );
    assert_eq!(
        (
            result.effort_basis.as_str(),
            result.estimator_prompt.as_str()
        ),
        ("basis", "prompt")
    );
}

#[rstest]
#[case::huge(f64::MAX)]
#[case::not_a_number(f64::NAN)]
fn rejects_hours_a_decimal_cannot_hold(#[case] hours: f64) {
    let source = report(vec![], vec![pull("dev", &[], Some(hours))]);
    let error = summarize(&source, &IndexMap::new()).unwrap_err();
    assert!(matches!(error, SummaryError::Hours { .. }), "{error}");
}

#[test]
fn reports_spend_overflow() {
    let mut row = spend("user-1", "", "1");
    row.spend = Decimal::MAX;
    let source = report(vec![row.clone(), row], vec![]);
    let error = summarize(&source, &IndexMap::new()).unwrap_err();
    assert!(
        matches!(error, SummaryError::Overflow { figure: "spend" }),
        "{error}"
    );
}

fn update_outcome(body: &str) -> String {
    match serde_json::from_str::<ROIIdentityMapUpdate>(body) {
        Ok(update) => format!("{update:?}"),
        Err(error) => error.to_string(),
    }
}

fn identity_map_json() -> String {
    let response = ROIIdentityMapResponse {
        report: None,
        identity_map: mappings(&[("octo", "octo@example.com")]),
    };
    serde_json::to_string(&response).unwrap()
}

fn report_json(source: Option<ROIReport>) -> String {
    let response = ROIReportResponse {
        report: source.map(|found| summary(&found)),
    };
    serde_json::to_string(&response).unwrap()
}

const REPORT_JSON: &str = r#"{"report":{"id":null,"mode":"live","start":"2026-01-01","end":"2026-01-08","synced_at":"2026-01-08T00:00:00Z","repos":["o/r"],"estimator_model":"model","estimator_prompt":"prompt","warnings":[],"effort_basis":"basis","metrics":{"matched_spend":2,"output_hours":3,"total_spend":2,"total_output_hours":3,"excluded_spend":0,"cost_per_hour":0.6666666666666666666666666667,"hours_per_dollar":1.5,"merged_prs":1,"estimated_prs":1,"matched_prs":1,"cohort_people":1,"people_with_prs":1,"pending_prs":0},"people":[{"id":"octo@example.com","email":"octo@example.com","logins":["Octo"],"spend":2,"hours":3,"prs":1,"estimated_prs":1,"pending_prs":0,"match_methods":["commit email"],"eligible":true,"cost_per_hour":0.6666666666666666666666666667}],"pulls":[{"repo":"o/r","number":1,"title":"t","url":"https://github.com/o/r/pull/1","login":"Octo","emails":["octo@example.com"],"profile_email":"home@example.com","merged_at":"2026-01-02T10:00:00Z","head_sha":"abc","additions":1,"deletions":2,"changed_files":3,"commit_count":4,"incomplete_metadata":false,"estimate":{"status":"estimated","hours":3,"reasoning":"r","model":null,"evidence_source":null,"effort_basis":null,"cached":false},"cache_key":null,"email":"octo@example.com","match_method":"commit email","matched":true}],"trend":[{"date":"2026-01-02","spend":2,"hours":3,"prs":1}]}}"#;

#[rstest]
#[case::update_normalizes_login(
    update_outcome(r#"{"github_login": " Octo-Cat ", "email": "Octo@Example.com"}"#),
    r#"ROIIdentityMapUpdate { github_login: "octo-cat", email: Some("Octo@Example.com") }"#
)]
#[case::update_null_email_removes(
    update_outcome(r#"{"github_login": "octo", "email": null}"#),
    r#"ROIIdentityMapUpdate { github_login: "octo", email: None }"#
)]
#[case::update_invalid_login(
    update_outcome(r#"{"github_login": "octo cat", "email": null}"#),
    "Enter a valid GitHub username."
)]
#[case::update_missing_email(
    update_outcome(r#"{"github_login": "octo"}"#),
    "missing field `email` at line 1 column 24"
)]
#[case::update_unknown_field(
    update_outcome(r#"{"github_login": "octo", "email": null, "extra": 1}"#),
    "unknown field `extra`, expected `github_login` or `email` at line 1 column 47"
)]
#[case::identity_map_response(
    identity_map_json(),
    r#"{"report":null,"identity_map":{"octo":"octo@example.com"}}"#
)]
#[case::empty_report_response(report_json(None), r#"{"report":null}"#)]
#[case::report_response_hides_commit_emails(
    report_json(Some(report(
        vec![spend("user-1", "octo@example.com", "2")],
        vec![with_profile(pull("Octo", &["octo@example.com"], Some(3.0_f64)), "home@example.com")],
    ))),
    REPORT_JSON
)]
fn identity_map_models(#[case] json: String, #[case] expected: &str) {
    assert_eq!(json, expected);
}

#[test]
fn replays_the_roi_identity_map_update_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/roi_identity_map_update");
    for entry in fs::read_dir(corpus).unwrap() {
        let path = entry.unwrap().path();
        let bytes = fs::read(&path).unwrap();
        let result = serde_json::from_slice::<ROIIdentityMapUpdate>(&bytes);
        let name = path.file_name().unwrap().to_str().unwrap();
        let expected_success = matches!(name, "valid.json" | "remove.json");
        assert_eq!(result.is_ok(), expected_success, "{name}");
    }
}
