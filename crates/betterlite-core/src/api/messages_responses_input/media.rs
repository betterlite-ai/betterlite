use super::items::MessagePart;
use super::records::{DocumentSource, ImageSource, PromptCacheBreakpoint};

fn data_url(media_type: &str, data: &str) -> String {
    let mut url = String::from("data:");
    url.push_str(media_type);
    url.push_str(";base64,");
    url.push_str(data);
    url
}

#[must_use]
pub(super) fn image_url(source: Option<&ImageSource>) -> Option<String> {
    match source {
        None | Some(ImageSource::Other) => None,
        Some(ImageSource::Base64 { data, .. }) if data.is_empty() => None,
        Some(ImageSource::Base64 { media_type, data }) => Some(data_url(
            media_type.as_deref().unwrap_or("image/jpeg"),
            data,
        )),
        Some(ImageSource::Url { url }) => url.clone().filter(|address| !address.is_empty()),
    }
}

#[must_use]
pub(super) fn document_part(
    source: Option<&DocumentSource>,
    title: Option<&str>,
    marker: Option<PromptCacheBreakpoint>,
) -> Option<MessagePart> {
    match source? {
        DocumentSource::Base64 { media_type, data } => {
            if data.is_empty() {
                return None;
            }
            let media = media_type
                .as_deref()
                .filter(|media| !media.is_empty())
                .unwrap_or("application/pdf");
            let filename = title
                .filter(|name| !name.is_empty())
                .unwrap_or("document.pdf");
            Some(MessagePart::FileData {
                filename: filename.to_owned(),
                file_data: data_url(media, data),
                prompt_cache_breakpoint: marker,
            })
        }
        DocumentSource::Url { url } => {
            let address = url.as_deref().filter(|value| !value.is_empty())?;
            Some(MessagePart::FileUrl {
                file_url: address.to_owned(),
                prompt_cache_breakpoint: marker,
            })
        }
        DocumentSource::Other => None,
    }
}
