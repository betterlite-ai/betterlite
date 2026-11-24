use super::super::cursor::{SessionCursor, decode_session_cursor};

fn cursor(last_activity: &str, session_key: &str, api_key: &str) -> SessionCursor {
    SessionCursor {
        last_activity: last_activity.to_owned(),
        session_key: session_key.to_owned(),
        api_key_hash: api_key.to_owned(),
    }
}

// the wire order is activity|api_key|session, and the split stops after the session begins
#[test]
fn splits_at_the_first_two_pipes() {
    assert_eq!(
        decode_session_cursor(Some("2026-01-01 00:00:00|hashed-key|session-1")),
        Some(cursor("2026-01-01 00:00:00", "session-1", "hashed-key"))
    );
    assert_eq!(
        decode_session_cursor(Some("2026-01-01 00:00:00|hashed-key|session|with|pipes")),
        Some(cursor(
            "2026-01-01 00:00:00",
            "session|with|pipes",
            "hashed-key"
        ))
    );
}

// an empty API key is a key position the caller can resume from
#[test]
fn keeps_an_empty_api_key() {
    assert_eq!(
        decode_session_cursor(Some("2026-01-01 00:00:00||session-1")),
        Some(cursor("2026-01-01 00:00:00", "session-1", ""))
    );
}

// _parse_session_cursor returns None when either separator is missing or the ends are empty
#[test]
fn malformed_cursors_are_no_cursor() {
    assert_eq!(decode_session_cursor(None), None);
    assert_eq!(decode_session_cursor(Some("")), None);
    assert_eq!(
        decode_session_cursor(Some("2026-01-01 00:00:00|hashed-key")),
        None
    );
    assert_eq!(decode_session_cursor(Some("|hashed-key|session-1")), None);
    assert_eq!(
        decode_session_cursor(Some("2026-01-01 00:00:00|hashed-key|")),
        None
    );
    assert_eq!(decode_session_cursor(Some("||")), None);
}

// the cursor carries the database's own text, and its time is only checked by the timestamp cast
#[test]
fn keeps_the_activity_text_as_written() {
    assert_eq!(
        decode_session_cursor(Some("not a timestamp|k|s")),
        Some(cursor("not a timestamp", "s", "k"))
    );
}
