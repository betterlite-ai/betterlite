#![cfg(test)]

use crate::api::video_contracts::config::BaseVideoConfig as _;

use super::fixture::Subject;

#[test]
fn multipart_default_is_false() {
    assert!(!Subject.use_multipart_form_data());
}
