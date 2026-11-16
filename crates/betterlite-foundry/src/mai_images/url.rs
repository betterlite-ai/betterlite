use super::error::MaiImageError;

const DEFAULT_API_VERSION: &str = "preview";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageOperation {
    Generation,
    Edit,
}

impl ImageOperation {
    const fn path_segment(self) -> &'static str {
        match self {
            Self::Generation => "generations",
            Self::Edit => "edits",
        }
    }

    const fn error_name(self) -> &'static str {
        match self {
            Self::Generation => "generation",
            Self::Edit => "editing",
        }
    }
}

// An api_base that already holds /mai/ is cut at its first /images/; otherwise /mai/v1/images/ is appended.
pub fn mai_image_url(
    operation: ImageOperation,
    api_base: Option<&str>,
    api_version: Option<&str>,
) -> Result<String, MaiImageError> {
    let base = api_base.ok_or_else(|| MaiImageError::ApiBaseRequired {
        operation: operation.error_name(),
    })?;
    let version = api_version
        .filter(|candidate| !candidate.is_empty())
        .unwrap_or(DEFAULT_API_VERSION);
    let (before_query, query) = base
        .split_once('?')
        .map_or((base, None), |(before, after)| (before, Some(after)));
    let trimmed = before_query.trim_end_matches('/');
    let segment = operation.path_segment();
    let path = if trimmed.contains("/mai/") {
        let prefix = trimmed
            .split_once("/images/")
            .map_or(trimmed, |(before, _)| before);
        format!("{prefix}/images/{segment}")
    } else {
        format!("{trimmed}/mai/v1/images/{segment}")
    };
    // An original query is kept verbatim, an empty one included, so api-version is only injected without a ?
    Ok(query.map_or_else(
        || format!("{path}?api-version={version}"),
        |original| format!("{path}?{original}"),
    ))
}

#[must_use]
pub fn is_mai_model(model: &str) -> bool {
    model
        .chars()
        .filter(|character| !matches!(character, '-' | '_'))
        .flat_map(char::to_lowercase)
        .collect::<String>()
        .contains("maiimage")
}
