#![cfg(test)]

use super::NoQueueStream;
use crate::sse_data_generator::types::{
    FailureKind, FailureParam, FailureType, StreamErrorState as _, StreamFailure,
};

fn failure() -> StreamFailure {
    StreamFailure {
        message: "upstream broke".to_owned(),
        error_type: FailureType::Text("upstream_error".to_owned()),
        param: FailureParam::Null,
        status_code: Some(500),
        code: None,
        body: None,
        kind: FailureKind::Other,
    }
}

#[test]
fn the_idle_error_state_leaves_a_frame_as_it_is() {
    let mut state = NoQueueStream;
    assert_eq!(
        state.mark_emitted("data: {}\n\n".to_owned()),
        "data: {}\n\n"
    );
}

#[test]
fn the_idle_error_state_renders_no_failure_frame() {
    let mut state = NoQueueStream;
    assert!(state.format_failure(&failure()).is_none());
}
