use rstest::rstest;

use super::{MODEL, OPERATION, VIDEO_ID, object, text};
use crate::vertex_veo_operation::error::VeoOperationError;
use crate::vertex_veo_operation::request::fetch_operation_request;

const API_BASE: &str = "https://us-central1-aiplatform.googleapis.com/v1/projects/p/locations/l/publishers/google/models";

#[test]
fn a_poll_reads_the_operation_from_the_video_id() {
    let request = fetch_operation_request(VIDEO_ID, API_BASE).expect("the video id decodes");
    assert_eq!(
        request.url,
        format!("{API_BASE}/{MODEL}:fetchPredictOperation")
    );
    assert_eq!(request.params, object([("operationName", text(OPERATION))]));
}

#[test]
fn a_trailing_slash_on_the_api_base_is_dropped() {
    let request =
        fetch_operation_request(VIDEO_ID, &format!("{API_BASE}//")).expect("the video id decodes");
    assert_eq!(
        request.url,
        format!("{API_BASE}/{MODEL}:fetchPredictOperation")
    );
}

#[rstest]
#[case::no_components("operations/op-1")]
#[case::seven_components("projects/p/locations/l/publishers/google/models")]
#[case::empty_model("projects/p/locations/l/publishers/google/models//operations/op-1")]
fn an_operation_without_a_model_is_rejected(#[case] video_id: &str) {
    let error = fetch_operation_request(video_id, API_BASE).expect_err("the model is missing");
    assert!(matches!(
        &error,
        VeoOperationError::InvalidOperation { operation } if operation.as_str() == video_id
    ));
    assert_eq!(
        error.to_string(),
        format!(
            "Invalid operation name format: {video_id}. Expected format: projects/PROJECT/locations/LOCATION/publishers/google/models/MODEL/operations/OPERATION_ID"
        )
    );
}
