#![cfg(test)]

use crate::api::video_records::VideoObject;
use crate::api::video_records::dict_access::DictAccess;

use super::fixture::{full_video, video};

#[test]
fn contains_reports_every_declared_video_field() {
    let video = VideoObject::default();
    for field in VideoObject::FIELDS {
        assert!(video.contains(field), "missing {field}");
    }
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
