#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionCursor {
    pub last_activity: String,
    pub session_key: String,
    // the stored key is a hash, so the name says so
    pub api_key_hash: String,
}

// _parse_session_cursor: time|api_key|session split at the first two pipes, so an empty key and pipes inside the session survive, while a missing separator or an empty time or session is no cursor
#[must_use]
pub fn decode_session_cursor(session_cursor: Option<&str>) -> Option<SessionCursor> {
    let mut parts = session_cursor?.splitn(3, '|');
    let last_activity = parts.next()?;
    let api_key = parts.next()?;
    let session_key = parts.next()?;
    if last_activity.is_empty() || session_key.is_empty() {
        return None;
    }
    Some(SessionCursor {
        last_activity: last_activity.to_owned(),
        session_key: session_key.to_owned(),
        api_key_hash: api_key.to_owned(),
    })
}
