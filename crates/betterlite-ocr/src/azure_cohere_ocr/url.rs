use super::AZURE_COHERE_PARSE_PATH;
use crate::azure_ai_ocr::url::{api_base_error, complete_provider_path};
use crate::ocr_error::OcrError;
use url::Url;

pub(crate) fn complete_parse_url(base: &str) -> Result<String, OcrError> {
    let mut url = Url::parse(base.trim()).map_err(|_source| api_base_error())?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(api_base_error());
    }
    let path = url.path().trim_end_matches('/').to_owned();
    if path.ends_with("/v2/parse") {
        url.set_path(&path);
        return Ok(String::from(url));
    }
    url.set_path(path.strip_suffix("/models").unwrap_or(path.as_str()));
    complete_provider_path(url.as_ref(), &AZURE_COHERE_PARSE_PATH)
}
