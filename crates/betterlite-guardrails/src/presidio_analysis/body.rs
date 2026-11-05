use super::error::PresidioAnalysisError;

pub(super) async fn read_capped_body(
    mut response: reqwest::Response,
    limit: usize,
) -> Result<Vec<u8>, PresidioAnalysisError> {
    let limit_bytes = u64::try_from(limit).unwrap_or(u64::MAX);
    if response
        .content_length()
        .is_some_and(|length| length > limit_bytes)
    {
        return Err(PresidioAnalysisError::ResponseTooLarge { limit });
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|source| PresidioAnalysisError::Request { source })?
    {
        body.extend_from_slice(&chunk);
        if body.len() > limit {
            return Err(PresidioAnalysisError::ResponseTooLarge { limit });
        }
    }
    Ok(body)
}

// Reads only as far as the first max_chars characters need, so a huge error page costs a few chunks.
pub(super) async fn read_text_prefix(
    mut response: reqwest::Response,
    max_chars: usize,
) -> Result<String, PresidioAnalysisError> {
    let max_bytes = max_chars.saturating_mul(4);
    let mut body = Vec::new();
    while body.len() < max_bytes {
        let Some(chunk) = response
            .chunk()
            .await
            .map_err(|source| PresidioAnalysisError::Request { source })?
        else {
            break;
        };
        body.extend_from_slice(&chunk);
    }
    let mut text = String::new();
    for part in body.utf8_chunks() {
        text.push_str(part.valid());
        if !part.invalid().is_empty() {
            text.push(char::REPLACEMENT_CHARACTER);
        }
    }
    Ok(text.chars().take(max_chars).collect())
}
