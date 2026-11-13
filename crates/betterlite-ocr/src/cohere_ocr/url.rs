use crate::ocr_error::OcrError;
use url::Url;

pub(crate) fn complete_parse_url(api_base: &str) -> Result<String, OcrError> {
    let mut url = Url::parse(api_base).map_err(|_source| OcrError::RequestField {
        path: "api_base".to_owned(),
    })?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(OcrError::RequestField {
            path: "api_base".to_owned(),
        });
    }
    let existing: Vec<String> = url
        .path_segments()
        .ok_or_else(|| OcrError::RequestField {
            path: "api_base".to_owned(),
        })?
        .filter(|segment| !segment.is_empty())
        .map(str::to_owned)
        .collect();
    let target = ["v2", "parse"];
    let overlap = path_overlap(&existing, &target);
    url.path_segments_mut()
        .map_err(|_source| OcrError::RequestField {
            path: "api_base".to_owned(),
        })?
        .pop_if_empty()
        .extend(target.iter().copied().skip(overlap));
    Ok(url.to_string())
}

fn path_overlap(existing: &[String], target: &[&str]) -> usize {
    let longest = existing.len().min(target.len());
    let mut overlap = 0;
    for length in 1..=longest {
        let suffix = existing
            .get(existing.len().saturating_sub(length)..)
            .unwrap_or_default();
        let prefix = target.get(..length).unwrap_or_default();
        if suffix.iter().map(String::as_str).eq(prefix.iter().copied()) {
            overlap = length;
        }
    }
    overlap
}
