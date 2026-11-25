#![cfg(test)]

use super::input::Input;
use betterlite_models::decisions::Json;
use rstest::rstest;

#[rstest]
#[case::at_count(2, true)]
#[case::over_count(1, false)]
fn managed_count_is_bounded(#[case] cap: usize, #[case] accepted: bool) {
    let input: Input = serde_json::from_str(
        r#"{
        "setup":{"model":"m","kwargs":{},"first_message":"first"},
        "frames":["second"],"disconnect":"closed","scripts":[],
        "quota":null,"sink":{"fails_at":null,"message":"send"}
    }"#,
    )
    .unwrap();
    assert_eq!(input.check_bounds(cap, 4096, 4096).is_ok(), accepted);
}

#[rstest]
#[case::at_capture(0, true)]
#[case::over_capture(1, false)]
fn managed_capture_is_bounded(#[case] reduction: usize, #[case] accepted: bool) {
    let input: Input = serde_json::from_str(
        r#"{
        "setup":{"model":"m","kwargs":{}},"frames":[],"disconnect":"closed",
        "scripts":[],"quota":null,"sink":{"fails_at":null,"message":"send"}
    }"#,
    )
    .unwrap();
    let size = serde_json::to_vec(&input).unwrap().len();
    assert_eq!(
        input.check_bounds(1024, 4096, size - reduction).is_ok(),
        accepted
    );
}

#[rstest]
#[case::at_frame(512, true)]
#[case::over_frame(511, false)]
fn managed_frame_is_bounded(#[case] cap: usize, #[case] accepted: bool) {
    let mut input: Input = serde_json::from_str(
        r#"{
        "setup":{"model":"m","kwargs":{}},"frames":[],"disconnect":"closed",
        "scripts":[],"quota":null,"sink":{"fails_at":null,"message":"send"}
    }"#,
    )
    .unwrap();
    input.frames.push("x".repeat(512));
    assert_eq!(input.check_bounds(1024, cap, 4096).is_ok(), accepted);
}

#[rstest]
#[case::at_chunk(512, true)]
#[case::over_chunk(511, false)]
fn managed_provider_chunk_is_bounded(#[case] cap: usize, #[case] accepted: bool) {
    let mut input: Input = serde_json::from_str(
        r#"{
        "setup":{"model":"m","kwargs":{}},"frames":[],"disconnect":"closed",
        "scripts":[{"chunks":[{"kind":"object","dump":"payload"}],"raise_message":""}],
        "quota":null,"sink":{"message":"send"}
    }"#,
    )
    .unwrap();
    input.scripts[0].chunks[0].dump = Some("x".repeat(512));
    assert_eq!(input.check_bounds(1024, cap, 4096).is_ok(), accepted);
    input.scripts[0].chunks[0].dump = None;
    input.scripts[0].chunks[0].value = Some(Json::Text("x".repeat(510)));
    assert_eq!(input.check_bounds(1024, cap, 4096).is_ok(), accepted);
}

#[rstest]
#[case::zero(0, false)]
#[case::at_deadline(120_000, true)]
#[case::over_deadline(120_001, false)]
fn managed_deadline_is_bounded(#[case] millis: u64, #[case] accepted: bool) {
    let mut input: Input = serde_json::from_str(
        r#"{
        "setup":{"model":"m","kwargs":{}},"frames":[],"disconnect":"closed",
        "scripts":[],"quota":null,"sink":{"message":"send"}
    }"#,
    )
    .unwrap();
    input.deadline_millis = millis;
    assert_eq!(input.check_bounds(1024, 4096, 4096).is_ok(), accepted);
}
