#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use super::super::query::{
    BenchmarksQuery, ReadQueryError, SessionQuery, decode_benchmarks_query, decode_session_query,
};

fn missing_session() -> ReadQueryError {
    ReadQueryError::Invalid {
        parameter: "session_id",
        kind: "missing",
        msg: "Field required",
    }
}

// both decoders read every corpus file, whichever read the bytes were seeded for
#[rstest]
fn replays_the_auto_router_read_api_corpus(
    #[files("fuzz/corpus/auto_router_read_api/*")] path: PathBuf,
) {
    let bytes = fs::read(&path).unwrap();
    let name = path.file_name().and_then(|name| name.to_str()).unwrap();
    let benchmarks = decode_benchmarks_query(&bytes);
    let session = decode_session_query(&bytes);
    match name {
        "benchmarks-valid" => {
            assert_eq!(session.unwrap_err(), missing_session());
            let query = benchmarks.unwrap();
            assert_eq!(query.start_date.as_deref(), Some("2024-01-01"));
            assert_eq!(query.end_date.as_deref(), Some("2024-01-02"));
            assert_eq!(query.key_hash.as_deref(), Some("hash-a"));
            assert_eq!(query.user_id.as_deref(), Some("u1"));
        }
        "benchmarks-empty-user" => assert_empty_user(benchmarks, session),
        "session-valid" => assert_session_ok(benchmarks, session, "s1"),
        "session-missing" => {
            assert_eq!(session.unwrap_err(), missing_session());
            assert_eq!(
                benchmarks.unwrap().start_date.as_deref(),
                Some("2024-01-01")
            );
        }
        "session-repeated" => assert_session_ok(benchmarks, session, "last"),
        "session-escaped" => assert_session_ok(benchmarks, session, "it's"),
        other => panic!("unexpected corpus file {other}"),
    }
}

fn assert_empty_user(
    benchmarks: Result<BenchmarksQuery, ReadQueryError>,
    session: Result<SessionQuery, ReadQueryError>,
) {
    assert_eq!(session.unwrap_err(), missing_session());
    assert_eq!(
        benchmarks.unwrap_err(),
        ReadQueryError::Invalid {
            parameter: "user_id",
            kind: "string_too_short",
            msg: "String should have at least 1 character",
        }
    );
}

fn assert_session_ok(
    benchmarks: Result<BenchmarksQuery, ReadQueryError>,
    session: Result<SessionQuery, ReadQueryError>,
    expected: &str,
) {
    assert_eq!(session.unwrap().session_id, expected);
    assert_eq!(benchmarks.unwrap(), BenchmarksQuery::default());
}
