#![cfg(test)]

use crate::api::video_contracts::config::BaseVideoConfig as _;

use super::fixture::{Parts, Subject};

#[test]
fn prefetch_default_is_absent() {
    let parts = Parts::default();
    assert_eq!(
        Subject.get_video_edit_prefetch_params("video_1", parts.context()),
        None
    );
}
