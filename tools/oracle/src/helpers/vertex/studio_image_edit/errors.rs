use betterlite_oracle_helper_api::error::HelperError;
use betterlite_vertex::studio_image_edit::error::StudioImageEditError;

// the status LiteLLM's exception carries: the imageConfig 400, or the status of the reply that did not parse
pub(super) fn edit_failure(error: &StudioImageEditError) -> Box<HelperError> {
    let status = error
        .status()
        .or_else(|| error.upstream_answer().map(|(status, _headers)| status));
    Box::new(HelperError {
        class: "ValueError".to_owned(),
        status,
        error_type: Some(None),
        param: Some(None),
        code: Some(None),
        message: error.to_string(),
    })
}
