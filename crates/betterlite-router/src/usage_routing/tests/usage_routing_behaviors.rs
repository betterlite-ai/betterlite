use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use rstest::rstest;

use crate::deployment_identity::spec::DeploymentSpec;
use crate::router_predicates::JsonMap;
use crate::usage_routing::record::{SuccessUse, record_success};
use crate::usage_routing::select::{WindowCounts, select, select_counts};
use crate::usage_routing::window::window_keys;

use super::support::{LocalWindows, counts, instant, number, plain, success_params, text};

const GROUP: &str = "gpt-4o";
// 2023-11-14T22:13:20Z, the minute the window keys label 22-13
const NOW: i64 = 1_700_000_000_000;

#[derive(Clone, Copy)]
enum Targets {
    First,
    Second,
    NumericId,
    NoMetadata,
    MetadataIsText,
    NullGroup,
    AbsentGroup,
    NullId,
    NoModelInfo,
    EmptyModelInfo,
    ModelInfoIsText,
}

impl Targets {
    fn id(self) -> Json {
        match self {
            Self::Second => text("dep-b"),
            Self::NumericId => number(42),
            Self::First
            | Self::NoMetadata
            | Self::MetadataIsText
            | Self::NullGroup
            | Self::AbsentGroup
            | Self::NullId
            | Self::NoModelInfo
            | Self::EmptyModelInfo
            | Self::ModelInfoIsText => text("dep-a"),
        }
    }

    fn params(self) -> JsonMap {
        let mut built = success_params(GROUP, self.id());
        match self {
            Self::NoMetadata => {
                built.shift_remove("metadata");
            }
            Self::MetadataIsText => {
                built.insert("metadata".to_owned(), text("x"));
            }
            Self::NullGroup => {
                built.insert(
                    "metadata".to_owned(),
                    Json::Object(JsonObject::from_iter([(
                        "model_group".to_owned(),
                        Json::Null,
                    )])),
                );
            }
            Self::AbsentGroup => {
                built.insert("metadata".to_owned(), Json::Object(JsonObject::default()));
            }
            Self::NullId => {
                built.insert(
                    "model_info".to_owned(),
                    Json::Object(JsonObject::from_iter([("id".to_owned(), Json::Null)])),
                );
            }
            Self::NoModelInfo => {
                built.shift_remove("model_info");
            }
            Self::EmptyModelInfo => {
                built.insert("model_info".to_owned(), Json::Object(JsonObject::default()));
            }
            Self::ModelInfoIsText => {
                built.insert("model_info".to_owned(), text("x"));
            }
            Self::First | Self::Second | Self::NumericId => {}
        }
        built
    }
}

fn record(windows: &LocalWindows, targets: Targets, call_type: Option<&str>, tokens: Option<i64>) {
    let built = targets.params();
    let success = SuccessUse {
        call_type,
        litellm_params: &built,
        total_tokens: tokens,
    };
    record_success(windows, 60, instant(NOW), &success);
}

struct RecordCase {
    targets: Targets,
    call_type: Option<&'static str>,
    tokens: Option<i64>,
    seed: &'static [(&'static str, i64)],
    tokens_window: Option<&'static [(&'static str, i64)]>,
    requests_window: Option<&'static [(&'static str, i64)]>,
}

// log_success_event and async_log_success_event share one read-modify-write, so each case is one success and
// the two windows it leaves behind.
#[rstest]
#[case::plain(RecordCase { targets: Targets::First, call_type: None, tokens: Some(120), seed: &[], tokens_window: Some(&[("dep-a", 120)]), requests_window: Some(&[("dep-a", 1)]) })]
#[case::completion_call_type(RecordCase { targets: Targets::First, call_type: Some("acompletion"), tokens: Some(120), seed: &[], tokens_window: Some(&[("dep-a", 120)]), requests_window: Some(&[("dep-a", 1)]) })]
#[case::batch_retrieve(RecordCase { targets: Targets::First, call_type: Some("retrieve_batch"), tokens: Some(120), seed: &[], tokens_window: None, requests_window: None })]
#[case::async_batch_retrieve(RecordCase { targets: Targets::First, call_type: Some("aretrieve_batch"), tokens: Some(120), seed: &[], tokens_window: None, requests_window: None })]
#[case::zero_tokens(RecordCase { targets: Targets::First, call_type: None, tokens: Some(0), seed: &[], tokens_window: Some(&[("dep-a", 0)]), requests_window: Some(&[("dep-a", 1)]) })]
#[case::no_usage(RecordCase { targets: Targets::First, call_type: None, tokens: None, seed: &[], tokens_window: None, requests_window: None })]
#[case::numeric_id(RecordCase { targets: Targets::NumericId, call_type: None, tokens: Some(120), seed: &[], tokens_window: Some(&[("42", 120)]), requests_window: Some(&[("42", 1)]) })]
#[case::no_metadata(RecordCase { targets: Targets::NoMetadata, call_type: None, tokens: Some(120), seed: &[], tokens_window: None, requests_window: None })]
#[case::metadata_is_text(RecordCase { targets: Targets::MetadataIsText, call_type: None, tokens: Some(120), seed: &[], tokens_window: None, requests_window: None })]
#[case::null_group(RecordCase { targets: Targets::NullGroup, call_type: None, tokens: Some(120), seed: &[], tokens_window: None, requests_window: None })]
#[case::absent_group(RecordCase { targets: Targets::AbsentGroup, call_type: None, tokens: Some(120), seed: &[], tokens_window: None, requests_window: None })]
#[case::null_id(RecordCase { targets: Targets::NullId, call_type: None, tokens: Some(120), seed: &[], tokens_window: None, requests_window: None })]
#[case::no_model_info(RecordCase { targets: Targets::NoModelInfo, call_type: None, tokens: Some(120), seed: &[], tokens_window: None, requests_window: None })]
#[case::model_info_is_text(RecordCase { targets: Targets::ModelInfoIsText, call_type: None, tokens: Some(120), seed: &[], tokens_window: None, requests_window: None })]
#[case::existing_window(RecordCase { targets: Targets::First, call_type: None, tokens: Some(30), seed: &[("dep-a", 120)], tokens_window: Some(&[("dep-a", 150)]), requests_window: Some(&[("dep-a", 1)]) })]
#[case::another_deployment(RecordCase { targets: Targets::Second, call_type: None, tokens: Some(30), seed: &[("dep-a", 120)], tokens_window: Some(&[("dep-a", 120), ("dep-b", 30)]), requests_window: Some(&[("dep-b", 1)]) })]
#[case::empty_model_info(RecordCase { targets: Targets::EmptyModelInfo, call_type: None, tokens: Some(120), seed: &[], tokens_window: None, requests_window: None })]
fn usage_routing_behaviors(#[case] case: RecordCase) {
    let windows = LocalWindows::default();
    let (tokens_key, requests_key) = window_keys(GROUP, instant(NOW));
    windows.seed(&tokens_key, case.seed);
    record(&windows, case.targets, case.call_type, case.tokens);
    assert_eq!(windows.window(&tokens_key), case.tokens_window.map(counts));
    assert_eq!(
        windows.window(&requests_key),
        case.requests_window.map(counts)
    );
    let expected_ttl = case.tokens_window.map(|_| 60);
    assert_eq!(windows.ttl(&tokens_key), expected_ttl);
    assert_eq!(windows.ttl(&requests_key), expected_ttl);
}

// get_available_deployments over a stored window: the pooled id with the lowest count wins, and a pooled id
// the window never saw starts at zero.
#[rstest]
#[case::cold_window(&[], &["dep-a", "dep-b"], Some(0))]
#[case::lowest_count(&[("dep-a", 100), ("dep-b", 50)], &["dep-a", "dep-b"], Some(1))]
#[case::unrecorded_appends_at_zero(&[("dep-a", 100)], &["dep-a", "dep-b"], Some(1))]
#[case::removed_deployment_is_skipped(&[("dep-a", 0), ("dep-b", 50)], &["dep-b"], Some(0))]
#[case::removed_deployment_keeps_its_count(&[("dep-a", 0), ("dep-b", 50)], &["dep-a", "dep-b"], Some(0))]
fn usage_routing_select_behaviors(
    #[case] seed: &[(&str, i64)],
    #[case] pool: &[&str],
    #[case] expected: Option<usize>,
) {
    let windows = LocalWindows::default();
    let (tokens_key, _requests_key) = window_keys(GROUP, instant(NOW));
    windows.seed(&tokens_key, seed);
    let deployments = pool.iter().map(|id| plain(id)).collect::<Vec<_>>();
    assert_eq!(
        select(&windows, GROUP, &deployments, 0, instant(NOW)).unwrap(),
        expected
    );
}

#[test]
fn two_successes_accumulate_in_one_window() {
    let windows = LocalWindows::default();
    let (tokens_key, requests_key) = window_keys(GROUP, instant(NOW));
    record(&windows, Targets::First, None, Some(120));
    record(&windows, Targets::First, None, Some(30));
    assert_eq!(windows.window(&tokens_key), Some(counts(&[("dep-a", 150)])));
    assert_eq!(windows.window(&requests_key), Some(counts(&[("dep-a", 2)])));
}

// a stored count at the signed maximum saturates, so the addition cannot wrap to a count below zero
#[test]
fn a_count_at_the_signed_maximum_stays_there() {
    let windows = LocalWindows::default();
    let (tokens_key, requests_key) = window_keys(GROUP, instant(NOW));
    windows.seed(&tokens_key, &[("dep-a", i64::MAX)]);
    record(&windows, Targets::First, None, Some(10));
    assert_eq!(
        windows.window(&tokens_key),
        Some(counts(&[("dep-a", i64::MAX)]))
    );
    assert_eq!(windows.window(&requests_key), Some(counts(&[("dep-a", 1)])));
}

#[rstest]
#[case::configured(45)]
#[case::zero(0)]
#[case::negative(-1)]
fn the_configured_ttl_reaches_both_writes(#[case] ttl_seconds: i64) {
    let windows = LocalWindows::default();
    let (tokens_key, requests_key) = window_keys(GROUP, instant(NOW));
    let built = success_params(GROUP, text("dep-a"));
    let success = SuccessUse {
        call_type: None,
        litellm_params: &built,
        total_tokens: Some(5),
    };
    record_success(&windows, ttl_seconds, instant(NOW), &success);
    assert_eq!(windows.ttl(&tokens_key), Some(ttl_seconds));
    assert_eq!(windows.ttl(&requests_key), Some(ttl_seconds));
}

// LiteLLM's cache reads a failed get as a cold window and writes the fresh map back; betterlite keeps the
// stored counts instead, which docs/deviations/B317.yaml records.
#[test]
fn a_failed_read_drops_the_update_instead_of_replacing_the_window() {
    let windows = LocalWindows::default();
    let (tokens_key, requests_key) = window_keys(GROUP, instant(NOW));
    windows.seed(&tokens_key, &[("dep-a", 100)]);
    windows.seed(&requests_key, &[("dep-a", 1)]);
    windows.fail_reads(true);
    record(&windows, Targets::Second, None, Some(50));
    assert_eq!(windows.window(&tokens_key), Some(counts(&[("dep-a", 100)])));
    assert_eq!(windows.window(&requests_key), Some(counts(&[("dep-a", 1)])));
}

// one window's failure does not skip the other window's update, the way each cache call fails on its own
#[test]
fn a_failed_token_read_still_records_the_request() {
    let windows = LocalWindows::default();
    let (tokens_key, requests_key) = window_keys(GROUP, instant(NOW));
    windows.seed(&tokens_key, &[("dep-a", 100)]);
    windows.fail_read_on(&tokens_key);
    record(&windows, Targets::Second, None, Some(50));
    assert_eq!(windows.window(&tokens_key), Some(counts(&[("dep-a", 100)])));
    assert_eq!(windows.window(&requests_key), Some(counts(&[("dep-b", 1)])));
}

#[test]
fn a_failed_token_write_still_records_the_request() {
    let windows = LocalWindows::default();
    let (tokens_key, requests_key) = window_keys(GROUP, instant(NOW));
    windows.seed(&tokens_key, &[("dep-a", 100)]);
    windows.fail_write_on(&tokens_key);
    record(&windows, Targets::Second, None, Some(50));
    assert_eq!(windows.window(&tokens_key), Some(counts(&[("dep-a", 100)])));
    assert_eq!(windows.window(&requests_key), Some(counts(&[("dep-b", 1)])));
}

#[test]
fn a_failed_write_keeps_the_previous_window() {
    let windows = LocalWindows::default();
    let (tokens_key, _requests_key) = window_keys(GROUP, instant(NOW));
    windows.seed(&tokens_key, &[("dep-a", 100)]);
    windows.fail_writes(true);
    record(&windows, Targets::First, None, Some(50));
    assert_eq!(windows.window(&tokens_key), Some(counts(&[("dep-a", 100)])));
    windows.fail_writes(false);
    record(&windows, Targets::First, None, Some(50));
    assert_eq!(windows.window(&tokens_key), Some(counts(&[("dep-a", 150)])));
}

// a window read the store fails is the cold window LiteLLM's cache reports, never a request error
#[test]
fn a_failed_read_selects_from_the_cold_window() {
    let windows = LocalWindows::default();
    let (tokens_key, _requests_key) = window_keys(GROUP, instant(NOW));
    windows.seed(&tokens_key, &[("dep-a", 100), ("dep-b", 50)]);
    let deployments = [plain("dep-a"), plain("dep-b")];
    assert_eq!(
        select(&windows, GROUP, &deployments, 0, instant(NOW)).unwrap(),
        Some(1)
    );
    windows.fail_reads(true);
    assert_eq!(
        select(&windows, GROUP, &deployments, 0, instant(NOW)).unwrap(),
        Some(0)
    );
}

// log_success_event's f"{model_group}:tpm:{datetime.now().strftime('%H-%M')}" pair, spelled out
#[test]
fn window_keys_name_the_group_and_the_utc_minute() {
    let (tokens_key, requests_key) = window_keys(GROUP, instant(NOW));
    assert_eq!(tokens_key, "gpt-4o:tpm:22-13");
    assert_eq!(requests_key, "gpt-4o:rpm:22-13");
}

// the minute in the key is what keeps a past window from deciding this one
#[rstest]
#[case::same_minute(30_000, Some(1))]
#[case::next_minute(60_000, Some(0))]
fn a_later_request_reads_the_window_of_its_own_minute(
    #[case] elapsed_millis: i64,
    #[case] expected: Option<usize>,
) {
    let windows = LocalWindows::default();
    record(&windows, Targets::First, None, Some(10));
    let deployments = [plain("dep-a"), plain("dep-b")];
    let at = NOW.saturating_add(elapsed_millis);
    assert_eq!(
        select(&windows, GROUP, &deployments, 0, instant(at)).unwrap(),
        expected
    );
}

#[test]
fn an_empty_pool_selects_nothing() {
    let windows = WindowCounts::default();
    let deployments: [DeploymentSpec; 0] = [];
    assert_eq!(select_counts(&windows, &deployments, 0).unwrap(), None);
}
