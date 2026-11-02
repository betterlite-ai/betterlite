const DATA_URL_PREFIX: &str = "data:";
const BASE64_MARKER: &str = ";base64,";

#[must_use]
pub fn extract_base64_data(image_url: &str) -> String {
    if image_url.starts_with(DATA_URL_PREFIX)
        && let Some((_, payload)) = image_url.split_once(BASE64_MARKER)
    {
        return payload.to_owned();
    }
    image_url.to_owned()
}

mod tests;
