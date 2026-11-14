use betterlite_prompts::arguments::json::Json;
use betterlite_prompts::media::inline::remote_urls_to_inline;
use betterlite_prompts::message_content::{Content, Message};

use crate::gemini_history::content::{CONTENT_TYPE, FORMAT, MIME_TYPE};
use crate::gemini_history::error::HistoryError;
use crate::gemini_history::media_source::text_of;
use crate::gemini_media::error::{MediaError, MediaField};
use crate::gemini_media::mime::MimeHints;
use crate::gemini_media::policy::{GoogleProvider, inlines_remote};
use crate::model_policy::capabilities::VertexCapabilityProvider;
use crate::studio_chat::input::image_url_fields;

use super::error::{BodyError, RequestError};

const IMAGE_URL: &str = "image_url";
const FILE: &str = "file";
const FILE_ID: &str = "file_id";

// LiteLLM downloads every remote media its predicate accepts; betterlite refuses the request without fetching
pub fn reject_inlined_media(
    provider: VertexCapabilityProvider,
    messages: &[Message],
) -> Result<(), RequestError> {
    let urls = remote_urls_to_inline(messages, &|media| {
        let hints = MimeHints {
            format: text_of(&media.fields, FORMAT),
            mime_type: text_of(&media.fields, MIME_TYPE),
            content_type: text_of(&media.fields, CONTENT_TYPE),
        };
        let google = match provider {
            VertexCapabilityProvider::Gemini => GoogleProvider::AiStudio,
            VertexCapabilityProvider::VertexAi | VertexCapabilityProvider::VertexAiBeta => {
                GoogleProvider::Vertex
            }
        };
        inlines_remote(google, &media.url, &hints)
    })?;
    let Some(url) = urls.first() else {
        return Ok(());
    };
    Err(RequestError::Body(BodyError::History(HistoryError::Media(
        MediaError::RemoteMediaRequired(media_field(messages, url)),
    ))))
}

fn media_field(messages: &[Message], url: &str) -> MediaField {
    for message in messages {
        let Some(Content::Parts(parts)) = &message.content else {
            continue;
        };
        for part in parts {
            if part.kind == IMAGE_URL && image_url_fields(&part.fields).url.as_deref() == Some(url)
            {
                return MediaField::ImageUrl;
            }
            let file_id = part
                .fields
                .get(FILE)
                .and_then(Json::as_object)
                .and_then(|file| file.get(FILE_ID))
                .and_then(Json::as_str);
            if part.kind == FILE && file_id == Some(url) {
                return MediaField::File;
            }
        }
    }
    MediaField::File
}
