use super::error::NovaEmbeddingsError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedDataUrl {
    pub media_type: String,
    pub data: String,
}

pub fn parse_nova_data_url(data_url: &str) -> Result<ParsedDataUrl, NovaEmbeddingsError> {
    let Some(rest) = data_url.strip_prefix("data:") else {
        return Err(NovaEmbeddingsError::InvalidDataUrl {
            prefix: data_url.chars().take(50).collect(),
        });
    };
    let Some((metadata, data)) = rest.split_once(',') else {
        return Err(NovaEmbeddingsError::MissingComma {
            prefix: data_url.chars().take(50).collect(),
        });
    };
    let media_type = metadata.split(';').next().unwrap_or(metadata);
    Ok(ParsedDataUrl {
        media_type: media_type.to_owned(),
        data: data.to_owned(),
    })
}
