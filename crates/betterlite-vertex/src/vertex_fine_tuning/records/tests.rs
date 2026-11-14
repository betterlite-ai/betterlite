#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use betterlite_openai::fine_tuning::job::{FineTuneCreate, FineTuneJob};
use jiff::tz::TimeZone;
use rstest::rstest;
use rust_decimal::Decimal;

use super::TuningRecordError;
use super::request::{OriginalHyperparameters, VertexTuningJobCreate, to_vertex_request};
use super::response::decode_vertex_tuning_job;
use super::timestamp::created_at;
use crate::provider_metadata::timestamp::LocalTimeContext;

fn create(text: &str) -> FineTuneCreate {
    serde_json::from_str(text).unwrap()
}

fn utc() -> LocalTimeContext {
    LocalTimeContext {
        zone: TimeZone::get("UTC").unwrap(),
    }
}

fn request(text: &str, adapter: Option<&str>) -> VertexTuningJobCreate {
    let original = OriginalHyperparameters {
        adapter_size: adapter.map(str::to_owned),
    };
    to_vertex_request(&create(text), &original).unwrap()
}

#[test]
fn a_request_keeps_the_vertex_names() {
    let text = serde_json::to_string(&request(
        r#"{"model":"gemini-1.5-flash-002","training_file":"gs://bucket/train.jsonl","suffix":"my-run","validation_file":"gs://bucket/valid.jsonl","hyperparameters":{"n_epochs":3,"learning_rate_multiplier":0.1,"batch_size":4}}"#,
        Some("ADAPTER_SIZE_ONE"),
    ))
    .unwrap();
    assert_eq!(
        text,
        r#"{"baseModel":"gemini-1.5-flash-002","supervisedTuningSpec":{"training_dataset_uri":"gs://bucket/train.jsonl","validation_dataset":"gs://bucket/valid.jsonl","hyperParameters":{"epoch_count":3,"learning_rate_multiplier":0.1,"adapter_size":"ADAPTER_SIZE_ONE"}},"tunedModelDisplayName":"my-run"}"#
    );
}

#[test]
fn a_null_suffix_stays_null_and_an_empty_validation_file_is_dropped() {
    let text = serde_json::to_string(&request(
        r#"{"model":"m","training_file":"f","suffix":null,"validation_file":""}"#,
        None,
    ))
    .unwrap();
    assert_eq!(
        text,
        r#"{"baseModel":"m","supervisedTuningSpec":{"training_dataset_uri":"f"},"tunedModelDisplayName":null}"#
    );
}

#[test]
fn an_empty_suffix_is_kept_as_sent() {
    let value = serde_json::to_value(request(
        r#"{"model":"m","training_file":"f","suffix":""}"#,
        None,
    ))
    .unwrap();
    assert_eq!(value["tunedModelDisplayName"], serde_json::json!(""));
}

#[rstest]
#[case::zero_epochs(r#"{"n_epochs":0}"#, None)]
#[case::zero_rate(r#"{"learning_rate_multiplier":0.0}"#, None)]
#[case::zero_integer_rate(r#"{"learning_rate_multiplier":0}"#, None)]
#[case::empty_epoch_text(r#"{"n_epochs":""}"#, None)]
#[case::empty_rate_text(r#"{"learning_rate_multiplier":""}"#, None)]
#[case::null_epochs(r#"{"n_epochs":null}"#, None)]
#[case::null_rate(r#"{"learning_rate_multiplier":null}"#, None)]
#[case::empty_adapter("{}", Some(""))]
#[case::null_hyperparameters("null", None)]
#[case::other_keys(r#"{"batch_size":4}"#, None)]
fn zero_valued_hyperparameters_are_omitted(
    #[case] hyperparameters: &str,
    #[case] adapter: Option<&str>,
) {
    let text =
        format!(r#"{{"model":"m","training_file":"f","hyperparameters":{hyperparameters}}}"#);
    let spec = request(&text, adapter).supervised_tuning_spec;
    assert_eq!(
        serde_json::to_string(&spec).unwrap(),
        r#"{"training_dataset_uri":"f"}"#
    );
}

#[rstest]
#[case::unspecified("ADAPTER_SIZE_UNSPECIFIED")]
#[case::one("ADAPTER_SIZE_ONE")]
#[case::four("ADAPTER_SIZE_FOUR")]
#[case::eight("ADAPTER_SIZE_EIGHT")]
#[case::sixteen("ADAPTER_SIZE_SIXTEEN")]
fn adapter_sizes_are_the_declared_literals(#[case] adapter: &str) {
    let value = serde_json::to_value(request(
        r#"{"model":"m","training_file":"f"}"#,
        Some(adapter),
    ))
    .unwrap();
    assert_eq!(
        value["supervisedTuningSpec"]["hyperParameters"]["adapter_size"],
        serde_json::json!(adapter)
    );
}

#[test]
fn an_unknown_adapter_size_is_kept_as_sent() {
    let value = serde_json::to_value(request(
        r#"{"model":"m","training_file":"f"}"#,
        Some("ADAPTER_SIZE_TWO"),
    ))
    .unwrap();
    assert_eq!(
        value["supervisedTuningSpec"]["hyperParameters"]["adapter_size"],
        serde_json::json!("ADAPTER_SIZE_TWO")
    );
}

#[test]
fn string_hyperparameters_are_parsed_and_a_text_zero_is_kept() {
    let sent = request(
        r#"{"model":"m","training_file":"f","hyperparameters":{"n_epochs":"0","learning_rate_multiplier":"0.25"}}"#,
        None,
    )
    .supervised_tuning_spec
    .hyperparameters;
    assert_eq!(
        serde_json::to_string(&sent).unwrap(),
        r#"{"epoch_count":0,"learning_rate_multiplier":0.25}"#
    );
}

#[rstest]
#[case::text_exponent(r#"{"learning_rate_multiplier":"2e-5"}"#)]
#[case::number_exponent(r#"{"learning_rate_multiplier":2e-05}"#)]
#[case::padded_text_exponent(r#"{"learning_rate_multiplier":" 2E-5 "}"#)]
fn an_exponent_rate_is_read_like_pythons_float(#[case] hyperparameters: &str) {
    let text =
        format!(r#"{{"model":"m","training_file":"f","hyperparameters":{hyperparameters}}}"#);
    let rate = request(&text, None)
        .supervised_tuning_spec
        .hyperparameters
        .unwrap()
        .learning_rate_multiplier;
    assert_eq!(rate, Some(Decimal::new(2, 5)));
}

#[rstest]
#[case::text_epochs(r#"{"n_epochs":"auto"}"#, "n_epochs")]
#[case::float_epochs(r#"{"n_epochs":3.0}"#, "n_epochs")]
#[case::text_rate(r#"{"learning_rate_multiplier":"auto"}"#, "learning_rate_multiplier")]
#[case::bool_rate(r#"{"learning_rate_multiplier":true}"#, "learning_rate_multiplier")]
fn a_hyperparameter_of_the_wrong_type_names_its_field(
    #[case] hyperparameters: &str,
    #[case] field: &str,
) {
    let text =
        format!(r#"{{"model":"m","training_file":"f","hyperparameters":{hyperparameters}}}"#);
    let error = to_vertex_request(
        &create(&text),
        &OriginalHyperparameters { adapter_size: None },
    )
    .unwrap_err();
    assert!(error.to_string().contains(field), "{error}");
}

#[test]
fn a_full_response_maps_every_openai_field() {
    let job = decode_vertex_tuning_job(
        br#"{"name":"projects/p/locations/us-central1/tuningJobs/1","tunedModelDisplayName":"tuned-1","baseModel":"gemini-1.5-flash-002","supervisedTuningSpec":{"trainingDatasetUri":"gs://b/train.jsonl","hyperParameters":{"epoch_count":3,"learning_rate_multiplier":0.1,"adapter_size":"ADAPTER_SIZE_FOUR"}},"state":"JOB_STATE_SUCCEEDED","createTime":"2024-01-01T00:00:00Z"}"#,
        &utc(),
    )
    .unwrap();
    let expected: FineTuneJob = serde_json::from_str(
        r#"{"id":"projects/p/locations/us-central1/tuningJobs/1","created_at":1704067200,"error":null,"fine_tuned_model":"tuned-1","finished_at":null,"hyperparameters":{"batch_size":null,"learning_rate_multiplier":0.1,"n_epochs":3,"adapter_size":"ADAPTER_SIZE_FOUR"},"model":"gemini-1.5-flash-002","object":"fine_tuning.job","organization_id":"","result_files":[],"seed":0,"status":"succeeded","trained_tokens":null,"training_file":"gs://b/train.jsonl","validation_file":null,"estimated_finish":null,"integrations":[],"metadata":null,"method":null}"#,
    )
    .unwrap();
    assert_eq!(
        serde_json::to_string(&job).unwrap(),
        serde_json::to_string(&expected).unwrap()
    );
}

#[test]
fn a_minimal_response_takes_the_source_defaults() {
    let job = decode_vertex_tuning_job(br#"{"state":"JOB_STATE_RUNNING"}"#, &utc()).unwrap();
    let value = serde_json::to_value(&job).unwrap();
    assert_eq!(value["id"], serde_json::json!(""));
    assert_eq!(value["model"], serde_json::json!(""));
    assert_eq!(value["fine_tuned_model"], serde_json::json!(""));
    assert_eq!(value["created_at"], serde_json::json!(0_i64));
    assert_eq!(value["training_file"], serde_json::json!(""));
    assert_eq!(value["seed"], serde_json::json!(0_i64));
    assert_eq!(value["object"], serde_json::json!("fine_tuning.job"));
    assert_eq!(value["organization_id"], serde_json::json!(""));
    assert_eq!(value["result_files"], serde_json::json!([]));
    assert_eq!(value["integrations"], serde_json::json!([]));
    let hyperparameters = &value["hyperparameters"];
    assert_eq!(hyperparameters["batch_size"], serde_json::json!(null));
    assert_eq!(
        hyperparameters["learning_rate_multiplier"],
        serde_json::json!(null)
    );
    assert_eq!(hyperparameters["n_epochs"], serde_json::json!(0_i64));
    assert!(hyperparameters.get("adapter_size").is_none());
}

#[test]
fn an_explicit_null_job_name_is_the_empty_string_and_a_null_model_name_stays_null() {
    let job = decode_vertex_tuning_job(
        br#"{"state":"JOB_STATE_FAILED","name":null,"baseModel":null,"tunedModelDisplayName":null}"#,
        &utc(),
    )
    .unwrap();
    let value = serde_json::to_value(&job).unwrap();
    assert_eq!(value["id"], serde_json::json!(""));
    assert_eq!(value["model"], serde_json::json!(""));
    assert_eq!(value["fine_tuned_model"], serde_json::json!(null));
}

#[rstest]
#[case::pending("JOB_STATE_PENDING", "queued")]
#[case::running("JOB_STATE_RUNNING", "running")]
#[case::succeeded("JOB_STATE_SUCCEEDED", "succeeded")]
#[case::failed("JOB_STATE_FAILED", "failed")]
#[case::cancelled("JOB_STATE_CANCELLED", "cancelled")]
#[case::unknown("JOB_STATE_PAUSED", "queued")]
#[case::empty_state("", "queued")]
fn states_map_to_the_openai_status(#[case] state: &str, #[case] status: &str) {
    let text = format!(r#"{{"state":"{state}"}}"#);
    let job = decode_vertex_tuning_job(text.as_bytes(), &utc()).unwrap();
    assert_eq!(
        serde_json::to_value(&job).unwrap()["status"],
        serde_json::json!(status)
    );
}

#[test]
fn a_null_state_queues_the_job() {
    let job = decode_vertex_tuning_job(br#"{"state":null}"#, &utc()).unwrap();
    assert_eq!(
        serde_json::to_value(&job).unwrap()["status"],
        serde_json::json!("queued")
    );
}

#[test]
fn a_missing_state_is_an_error() {
    let error = decode_vertex_tuning_job(br#"{"name":"n"}"#, &utc()).unwrap_err();
    assert!(matches!(error, TuningRecordError::MissingState));
}

#[rstest]
#[case::absent(r#"{"state":"JOB_STATE_RUNNING","supervisedTuningSpec":{"hyperParameters":{}}}"#)]
#[case::null_object(
    r#"{"state":"JOB_STATE_RUNNING","supervisedTuningSpec":{"hyperParameters":null}}"#
)]
#[case::absent_spec(r#"{"state":"JOB_STATE_RUNNING"}"#)]
fn an_absent_epoch_count_defaults_to_zero(#[case] text: &str) {
    let job = decode_vertex_tuning_job(text.as_bytes(), &utc()).unwrap();
    assert_eq!(
        serde_json::to_value(&job).unwrap()["hyperparameters"]["n_epochs"],
        serde_json::json!(0_i64)
    );
}

#[test]
fn a_null_epoch_count_leaves_n_epochs_null() {
    let job = decode_vertex_tuning_job(
        br#"{"state":"JOB_STATE_RUNNING","supervisedTuningSpec":{"hyperParameters":{"epoch_count":null}}}"#,
        &utc(),
    )
    .unwrap();
    assert_eq!(
        serde_json::to_value(&job).unwrap()["hyperparameters"]["n_epochs"],
        serde_json::json!(null)
    );
}

#[test]
fn a_null_adapter_size_is_kept_in_the_openai_record() {
    let job = decode_vertex_tuning_job(
        br#"{"state":"JOB_STATE_RUNNING","supervisedTuningSpec":{"hyperParameters":{"adapter_size":null}}}"#,
        &utc(),
    )
    .unwrap();
    assert_eq!(
        serde_json::to_value(&job).unwrap()["hyperparameters"]["adapter_size"],
        serde_json::json!(null)
    );
}

#[rstest]
#[case::zulu("2024-01-01T00:00:00Z", 1_704_067_200_i64)]
#[case::fraction("2024-01-01T00:00:00.123456Z", 1_704_067_200)]
#[case::positive_offset("2024-01-01T00:00:00+02:00", 1_704_060_000)]
#[case::negative_offset("2024-01-01T00:00:00-05:30", 1_704_087_000)]
#[case::date_only("2024-01-01", 1_704_067_200)]
#[case::space_separator("2024-01-01 03:04:05", 1_704_078_245)]
#[case::minutes_only("2024-01-01T03:04", 1_704_078_240)]
#[case::pre_epoch_fraction("1969-12-31T23:59:59.5Z", 0)]
#[case::pre_epoch_second_fraction("1969-12-31T23:59:58.5Z", -1)]
#[case::epoch_fraction("1970-01-01T00:00:00.9Z", 0)]
#[case::pre_epoch("1969-12-31T23:59:59Z", -1)]
#[case::first_year("0001-01-01T00:00:00Z", -62_135_596_800)]
#[case::last_second_jiff_holds("9999-12-30T21:59:59Z", 253_402_207_199)]
#[case::beyond_jiffs_range("9999-12-31T00:00:00Z", 0)]
#[case::double_zulu("2024-01-01T00:00:00ZZ", 0)]
#[case::empty("", 0)]
#[case::text("not-a-time", 0)]
#[case::bad_month("2024-13-01T00:00:00Z", 0)]
#[case::bad_hour("2024-01-01T25:00:00Z", 0)]
fn create_times_become_whole_unix_seconds(#[case] text: &str, #[case] expected: i64) {
    assert_eq!(created_at(Some(text), &utc()), expected);
}

#[test]
fn a_missing_create_time_is_zero() {
    assert_eq!(created_at(None, &utc()), 0);
}

#[rstest]
#[case::winter("2024-01-01T00:00:00", 1_704_063_600_i64)]
#[case::summer("2024-07-01T00:00:00", 1_719_784_800)]
fn a_naive_time_reads_the_injected_zone(#[case] text: &str, #[case] expected: i64) {
    let local = LocalTimeContext {
        zone: TimeZone::get("Europe/Berlin").unwrap(),
    };
    assert_eq!(created_at(Some(text), &local), expected);
}

#[rstest]
fn replays_the_vertex_tuning_job_corpus(#[files("fuzz/corpus/vertex_tuning_job/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    let _decoded = decode_vertex_tuning_job(&bytes, &utc());
}
