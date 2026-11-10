use crate::ocr_error::OcrError;
use url::Url;

pub(crate) fn complete_provider_path(base: &str, target: &[&str]) -> Result<String, OcrError> {
    let mut url = Url::parse(base.trim()).map_err(|_source| api_base_error())?;
    let existing: Vec<String> = url
        .path_segments()
        .ok_or_else(api_base_error)?
        .filter(|segment| !segment.is_empty())
        .map(str::to_owned)
        .collect();
    let overlap = path_overlap(&existing, target);
    url.path_segments_mut()
        .map_err(|()| api_base_error())?
        .pop_if_empty()
        .extend(target.iter().copied().skip(overlap));
    Ok(String::from(url))
}

pub(crate) fn api_base_error() -> OcrError {
    OcrError::RequestField {
        path: "api_base".to_owned(),
    }
}

fn path_overlap(existing: &[String], target: &[&str]) -> usize {
    let longest = existing.len().min(target.len());
    (0..=longest)
        .rev()
        .find(|length| {
            let suffix = existing
                .get(existing.len().saturating_sub(*length)..)
                .unwrap_or_default();
            suffix
                .iter()
                .map(String::as_str)
                .eq(target.iter().copied().take(*length))
        })
        .unwrap_or(0)
}
