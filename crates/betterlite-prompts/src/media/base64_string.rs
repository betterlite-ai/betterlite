use crate::media::data_url::is_canonical_base64;

const DATA_URL_PREFIX: &str = "data:";

// litellm.utils.get_base64_str: the second comma component, or the whole string when there is no comma
#[must_use]
pub fn get_base64_str(text: &str) -> &str {
    let Some(tail) = text.split_once(',').map(|(_head, tail)| tail) else {
        return text;
    };
    tail.split_once(',')
        .map_or(tail, |(component, _rest)| component)
}

// litellm.utils.is_base64_encoded: a data: string whose second comma component is canonical base64
#[must_use]
pub fn is_base64_encoded(text: &str) -> bool {
    let Some(rest) = text.strip_prefix(DATA_URL_PREFIX) else {
        return false;
    };
    let Some(payload) = rest.split(',').nth(1) else {
        return false;
    };
    is_canonical_base64(payload)
}
