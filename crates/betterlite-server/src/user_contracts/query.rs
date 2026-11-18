use percent_encoding::percent_decode_str;

pub const MAX_USER_ID_LENGTH: usize = 512;

// Python's len counts code points, and only U+0000..U+001F fail, so DEL and a space stay valid.
#[must_use]
pub fn is_valid_user_id(user_id: &str) -> bool {
    if user_id.chars().count() > MAX_USER_ID_LENGTH {
        return false;
    }
    user_id.chars().all(|character| character >= '\u{20}')
}

// The unanchored `user_id=([^&]*)` search: the first occurrence anywhere wins, even inside another value.
pub fn user_id_from_query(query: &str) -> Option<String> {
    let (_, captured) = query.split_once("user_id=")?;
    let value = match captured.split_once('&') {
        Some((value, _)) => value,
        None => captured,
    };
    let decoded = percent_decode_str(value).decode_utf8_lossy();
    if is_valid_user_id(decoded.as_ref()) {
        return Some(decoded.into_owned());
    }
    None
}
