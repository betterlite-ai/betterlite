use url::Url;

use crate::ocr_error::OcrError;

use super::REDUCTO_API_BASE;

pub(super) fn complete_endpoint_url(
    api_base: Option<&str>,
    path: &str,
) -> Result<String, OcrError> {
    let base = api_base
        .map(str::trim)
        .filter(|base| !base.is_empty())
        .unwrap_or(REDUCTO_API_BASE);
    let mut url = Url::parse(base).map_err(|_source| OcrError::RequestField {
        path: "api_base".to_owned(),
    })?;
    let segments: Vec<String> = url
        .path_segments()
        .map(|segments| {
            segments
                .filter(|segment| !segment.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .ok_or_else(|| OcrError::RequestField {
            path: "api_base".to_owned(),
        })?;
    let overlap = usize::from(segments.last().is_some_and(|segment| segment == path));
    {
        let mut target = url
            .path_segments_mut()
            .map_err(|()| OcrError::RequestField {
                path: "api_base".to_owned(),
            })?;
        target
            .pop_if_empty()
            .extend([path].get(overlap..).unwrap_or_default());
    };
    Ok(url.to_string())
}
