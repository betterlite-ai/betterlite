use crate::ocr_error::OcrError;
use crate::vertex_ocr::url::complete_url;

const DEFAULT_API_BASE: &str = "https://aiplatform.googleapis.com";

pub(crate) fn chat_completions_url(
    api_base: Option<&str>,
    project: &str,
    location: &str,
) -> Result<String, OcrError> {
    let target = [
        "v1",
        "projects",
        project,
        "locations",
        location,
        "endpoints",
        "openapi",
        "chat",
        "completions",
    ];
    complete_url(api_base, DEFAULT_API_BASE, &target)
}
