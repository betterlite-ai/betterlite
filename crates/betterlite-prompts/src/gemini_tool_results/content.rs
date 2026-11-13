use crate::anthropic_results::images::to_anthropic_image_chunk;
use crate::arguments::json::Json;
use crate::gemini_tool_results::error::GeminiToolResultsError;
use crate::gemini_tool_results::types::GeminiInlineData;
use crate::media::inline::RemoteMediaFetcher;
use crate::media::limits::MAX_MESSAGE_PARTS;
use crate::message_content::{Content, ContentPart};

const TEXT: &str = "text";
const IMAGE: &str = "image";
const INPUT_IMAGE: &str = "input_image";
const IMAGE_URL: &str = "image_url";
const FILE: &str = "file";
const INPUT_FILE: &str = "input_file";
const SOURCE: &str = "source";
const FILE_DATA: &str = "file_data";
const TYPE: &str = "type";
const URL: &str = "url";
const DATA: &str = "data";
const MEDIA_TYPE: &str = "media_type";
const BASE64: &str = "base64";
const DEFAULT_IMAGE_MEDIA_TYPE: &str = "image/jpeg";
const IMAGE_PREFIX: &str = "image/";
const DATA_URL_PREFIX: &str = "data:";
const BASE64_MARKER: &str = ";base64,";
const STRING: &str = "a string";
const STRING_OR_OBJECT: &str = "a string or an object";

pub(crate) struct CollectedContent {
    pub(crate) text: String,
    pub(crate) media: Vec<GeminiInlineData>,
}

pub(crate) async fn collect_content<Source>(
    content: Option<&Content>,
    fetcher: &Source,
) -> Result<CollectedContent, GeminiToolResultsError>
where
    Source: RemoteMediaFetcher + Sync,
{
    let mut collected = CollectedContent {
        text: String::new(),
        media: Vec::new(),
    };
    match content {
        None => {}
        Some(Content::Text(text)) => collect_text(text, &mut collected),
        Some(Content::Parts(parts)) => collect_parts(parts, fetcher, &mut collected).await?,
    }
    Ok(collected)
}

fn collect_text(text: &str, collected: &mut CollectedContent) {
    match promoted_data_url(text) {
        Some(media) => collected.media.push(media),
        None => collected.text.push_str(text),
    }
}

async fn collect_parts<Source>(
    parts: &[ContentPart],
    fetcher: &Source,
    collected: &mut CollectedContent,
) -> Result<(), GeminiToolResultsError>
where
    Source: RemoteMediaFetcher + Sync,
{
    if parts.len() > MAX_MESSAGE_PARTS {
        return Err(GeminiToolResultsError::TooManyParts {
            limit: MAX_MESSAGE_PARTS,
        });
    }
    for (index, part) in parts.iter().enumerate() {
        match part.kind.as_str() {
            TEXT => {
                let text = text_value(part.fields.get(TEXT), TEXT, STRING, index)?;
                collected.text.push_str(&text.unwrap_or_default());
            }
            IMAGE => {
                if let Some(media) = anthropic_image(part, index)? {
                    collected.media.push(media);
                }
            }
            INPUT_IMAGE | IMAGE_URL => {
                if let Some(url) = image_url(part, index)? {
                    append_converted_media(&url, fetcher, collected).await;
                }
            }
            FILE | INPUT_FILE => {
                if let Some(data) = file_data(part, index)? {
                    append_converted_media(&data, fetcher, collected).await;
                }
            }
            _ => {}
        }
    }
    Ok(())
}

// LiteLLM catches a failed download or a malformed data URL here and keeps the rest of the result
async fn append_converted_media<Source>(
    url_or_data: &str,
    fetcher: &Source,
    collected: &mut CollectedContent,
) where
    Source: RemoteMediaFetcher + Sync,
{
    match to_anthropic_image_chunk(url_or_data, None, fetcher).await {
        Ok(chunk) => collected.media.push(GeminiInlineData {
            data: chunk.data,
            mime_type: chunk.media_type,
        }),
        Err(error) => tracing::warn!(%error, "a tool result media part is dropped"),
    }
}

fn promoted_data_url(text: &str) -> Option<GeminiInlineData> {
    let prefix = text.get(..DATA_URL_PREFIX.len())?;
    if !prefix.eq_ignore_ascii_case(DATA_URL_PREFIX) || !text.contains(BASE64_MARKER) {
        return None;
    }
    let tail = text.get(DATA_URL_PREFIX.len()..)?;
    let (head, data) = tail.split_once(BASE64_MARKER)?;
    if !head.starts_with(IMAGE_PREFIX) {
        return None;
    }
    let media_type = head.split(';').next().unwrap_or(head).trim();
    Some(GeminiInlineData {
        data: data.to_owned(),
        mime_type: media_type.to_owned(),
    })
}

fn text_value(
    value: Option<&Json>,
    path: &str,
    expected: &'static str,
    index: usize,
) -> Result<Option<String>, GeminiToolResultsError> {
    match value {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text.clone())),
        Some(_) => Err(field_error(index, path, expected)),
    }
}

fn anthropic_image(
    part: &ContentPart,
    index: usize,
) -> Result<Option<GeminiInlineData>, GeminiToolResultsError> {
    let Some(Json::Object(source)) = part.fields.get(SOURCE) else {
        return Ok(None);
    };
    if source.get(TYPE).and_then(Json::as_str) != Some(BASE64) {
        return Ok(None);
    }
    let data = text_value(source.get(DATA), "source.data", STRING, index)?.unwrap_or_default();
    let mime_type = text_value(source.get(MEDIA_TYPE), "source.media_type", STRING, index)?
        .unwrap_or_else(|| DEFAULT_IMAGE_MEDIA_TYPE.to_owned());
    Ok(Some(GeminiInlineData { data, mime_type }))
}

fn image_url(part: &ContentPart, index: usize) -> Result<Option<String>, GeminiToolResultsError> {
    let value = match part.fields.get(IMAGE_URL) {
        Some(Json::Object(fields)) => text_value(fields.get(URL), "image_url.url", STRING, index)?,
        other => text_value(other, IMAGE_URL, STRING_OR_OBJECT, index)?,
    };
    Ok(value.filter(|url| !url.is_empty()))
}

fn file_data(part: &ContentPart, index: usize) -> Result<Option<String>, GeminiToolResultsError> {
    let direct = text_value(part.fields.get(FILE_DATA), FILE_DATA, STRING, index)?;
    if let Some(data) = direct.filter(|data| !data.is_empty()) {
        return Ok(Some(data));
    }
    match part.fields.get(FILE) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(data)) => Ok(non_empty(data)),
        Some(Json::Object(fields)) => {
            let nested = text_value(fields.get(FILE_DATA), "file.file_data", STRING, index)?;
            Ok(nested.filter(|data| !data.is_empty()))
        }
        Some(_) => Err(field_error(index, FILE, STRING_OR_OBJECT)),
    }
}

fn non_empty(text: &str) -> Option<String> {
    if text.is_empty() {
        return None;
    }
    Some(text.to_owned())
}

fn field_error(index: usize, path: &str, expected: &'static str) -> GeminiToolResultsError {
    GeminiToolResultsError::Field {
        field: format!("content[{index}].{path}"),
        expected,
    }
}
