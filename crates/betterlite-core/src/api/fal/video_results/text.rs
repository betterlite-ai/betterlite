use crate::api::fal::video_prepare::VideoHeaders;
use crate::api::fal::video_prepare::decode::{
    FalVideoDetail, FalVideoDetailItem, FalVideoLocation, FalVideoReply, decode_fal_video_reply,
};

// the detail field decides before the error field, and an empty detail string still wins over it
#[must_use]
pub fn error_text(reply: &FalVideoReply) -> Option<String> {
    match &reply.detail {
        Some(FalVideoDetail::Text(text)) => Some(text.clone()),
        Some(FalVideoDetail::Items(items)) => {
            let messages: Vec<String> = items.iter().filter_map(item_text).collect();
            if messages.is_empty() {
                return reply.error.clone();
            }
            Some(messages.join("; "))
        }
        None => reply.error.clone(),
    }
}

fn item_text(item: &FalVideoDetailItem) -> Option<String> {
    let message = item.msg.as_deref()?;
    match &item.loc {
        Some(FalVideoLocation::Text(location)) => Some(format!("{location}: {message}")),
        Some(FalVideoLocation::Parts(parts)) => Some(format!("{}: {message}", parts.join("."))),
        None => Some(message.to_owned()),
    }
}

// a failed answer keeps its structured error text, then its body, and names only the status when the body says nothing
#[must_use]
pub fn result_error(status: u16, body: &[u8]) -> Option<String> {
    if (200..=299).contains(&status) {
        return None;
    }
    // the decoder carries the reply headers into its own error alone, which the body fallback below discards
    let structured = decode_fal_video_reply(status, &VideoHeaders::new(), body)
        .ok()
        .and_then(|reply| error_text(&reply))
        .filter(|text| !text.is_empty());
    if let Some(text) = structured {
        return Some(text);
    }
    match str::from_utf8(body) {
        Ok(text) if !text.is_empty() => Some(text.to_owned()),
        Ok(_) | Err(_) => Some(format!("fal.ai returned HTTP {status}")),
    }
}

#[must_use]
pub fn terminal_result_error(status: u16, body: &[u8]) -> Option<String> {
    if status == 429 || status >= 500 {
        return None;
    }
    result_error(status, body)
}
