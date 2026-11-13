use url::Url;

use crate::ocr_error::OcrError;

pub(crate) fn raw_predict_url(
    api_base: Option<&str>,
    project: &str,
    location: &str,
    model: &str,
) -> Result<String, OcrError> {
    validate_location(location)?;
    let default_base = format!("https://{location}-aiplatform.googleapis.com");
    let prediction = format!("{model}:rawPredict");
    let target = [
        "v1",
        "projects",
        project,
        "locations",
        location,
        "publishers",
        "mistralai",
        "models",
        &prediction,
    ];
    complete_url(api_base, &default_base, &target)
}

fn validate_location(location: &str) -> Result<(), OcrError> {
    let valid = !location.is_empty()
        && location
            .bytes()
            .all(|value| value.is_ascii_lowercase() || value.is_ascii_digit() || value == b'-')
        && location
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
        && location
            .as_bytes()
            .last()
            .is_some_and(u8::is_ascii_alphanumeric);
    if valid {
        return Ok(());
    }
    Err(OcrError::RequestField {
        path: "vertex_location".to_owned(),
    })
}

pub(crate) fn complete_url(
    api_base: Option<&str>,
    default_base: &str,
    target: &[&str],
) -> Result<String, OcrError> {
    let request_field = || OcrError::RequestField {
        path: "api_base".to_owned(),
    };
    let base = api_base
        .map(str::trim)
        .filter(|base| !base.is_empty())
        .unwrap_or(default_base);
    let mut url = Url::parse(base).map_err(|_source| request_field())?;
    let existing: Vec<String> = url
        .path_segments()
        .ok_or_else(request_field)?
        .filter(|segment| !segment.is_empty())
        .map(str::to_owned)
        .collect();
    let overlap = (0..=existing.len().min(target.len()))
        .rev()
        .find(|&length| {
            existing
                .iter()
                .skip(existing.len().saturating_sub(length))
                .map(String::as_str)
                .eq(target.iter().copied().take(length))
        })
        .unwrap_or(0);
    url.path_segments_mut()
        .map_err(|()| request_field())?
        .pop_if_empty()
        .extend(target.iter().copied().skip(overlap));
    Ok(String::from(url))
}
