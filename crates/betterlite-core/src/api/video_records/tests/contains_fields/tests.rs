#![cfg(test)]

use rstest::rstest;

use crate::api::video_records::dict_access::DictAccess as _;

use super::fixture::{full_video, video};

#[rstest]
#[case::id("id")]
#[case::object("object")]
#[case::status("status")]
#[case::created_at("created_at")]
#[case::completed_at("completed_at")]
#[case::expires_at("expires_at")]
#[case::error("error")]
#[case::progress("progress")]
#[case::remixed_from_video_id("remixed_from_video_id")]
#[case::seconds("seconds")]
#[case::size("size")]
#[case::model("model")]
#[case::usage("usage")]
fn contains_reports_every_declared_video_field(#[case] field: &str) {
    assert!(video("video_123").contains(field), "missing {field}");
}

#[test]
fn contains_reports_the_hidden_params_field() {
    assert!(video("video_123").contains("_hidden_params"));
}

#[test]
fn contains_reports_a_set_optional_field() {
    assert!(full_video().contains("seconds"));
}

#[test]
fn contains_rejects_an_unknown_video_field() {
    let video = full_video();
    assert!(!video.contains("duration"));
    assert!(!video.contains(""));
}
