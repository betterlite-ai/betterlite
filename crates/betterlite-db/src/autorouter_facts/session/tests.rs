#![cfg(test)]

use super::{SESSION_ID_MAX_CHARS, bounded_session_id};

#[test]
fn an_id_at_the_limit_is_kept_as_sent() {
    let session_id = "a".repeat(SESSION_ID_MAX_CHARS);
    assert_eq!(bounded_session_id(&session_id), session_id);
}

#[test]
fn an_empty_id_is_kept() {
    assert_eq!(bounded_session_id(""), "");
}

#[test]
fn the_limit_counts_characters_not_bytes() {
    let session_id = "\u{fc}".repeat(SESSION_ID_MAX_CHARS);
    assert_eq!(bounded_session_id(&session_id), session_id);
}

#[test]
fn an_oversized_id_becomes_its_sha256_digest() {
    let session_id = "a".repeat(SESSION_ID_MAX_CHARS + 1);
    assert_eq!(
        bounded_session_id(&session_id),
        "sha256:e8d95cc2b4bc198c54b40bd214df958afb65f5e73d2c2eafe0593cf5c635c1f0"
    );
}

#[test]
fn an_oversized_multibyte_id_digests_its_utf8_bytes() {
    let session_id = "\u{fc}".repeat(SESSION_ID_MAX_CHARS + 1);
    assert_eq!(
        bounded_session_id(&session_id),
        "sha256:2af8c99ccdd8c38da1e1920a94e600700a4457cba16aa693f5903d298726d699"
    );
}

#[test]
fn the_digest_is_stable_so_one_session_stays_one_row() {
    let session_id = "b".repeat(SESSION_ID_MAX_CHARS + 100);
    assert_eq!(
        bounded_session_id(&session_id),
        bounded_session_id(&session_id)
    );
}
