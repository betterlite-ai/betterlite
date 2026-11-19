use sha2::{Digest as _, Sha256};

pub const SESSION_ID_MAX_CHARS: usize = 256;

// bounded_session_id: an oversized caller-chosen id maps to a stable digest, so its turns still aggregate into one session.
#[must_use]
pub fn bounded_session_id(session_id: &str) -> String {
    if session_id.chars().count() <= SESSION_ID_MAX_CHARS {
        return session_id.to_owned();
    }
    format!(
        "sha256:{}",
        hex::encode(Sha256::digest(session_id.as_bytes()))
    )
}

#[path = "session/tests.rs"]
mod tests;
