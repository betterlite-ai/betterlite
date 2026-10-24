use std::collections::BTreeMap;
use std::io::Read as _;
use std::time::Duration;
use std::{iter, thread};

use rstest::rstest;
use serde_json::Value;

use super::{OK, PAIR, SLOW, connect, script, seed, start};
use crate::mock::ProviderRequest;
use crate::routing_mock_scripts::{CallOutcome, MAX_TRACE, Recording, parse_script};

#[rstest]
#[case::replies_at_limit(script(10, &[OK; 256]), "")]
#[case::replies_past_limit(script(10, &[OK; 257]), "mock script has 257 replies, past the limit of 256")]
#[case::deadline_at_limit(script(30_000, &[OK]), "")]
#[case::deadline_past_limit(script(30_001, &[OK]), "mock script deadline_ms 30001 is outside 1 to 30000")]
#[case::deadline_zero(script(0, &[]), "mock script deadline_ms 0 is outside 1 to 30000")]
#[case::delay_at_deadline(script(40, &[SLOW]), "")]
#[case::delay_past_deadline(script(39, &[OK, SLOW]), "mock script reply 1 waits 40 ms, past the script deadline")]
#[case::body_and_events(script(10, &[r#"{"then":{"answer":{"status":200,"headers":{},"body":{},"events":[]}}}"#]), "mock script reply 0 must contain exactly one of body or events")]
#[case::status_out_of_range(script(10, &[r#"{"then":{"answer":{"status":1000,"headers":{},"body":{}}}}"#]), "mock script reply 0 has an invalid status")]
#[case::lone_barrier(script(10, &[PAIR]), "mock script barrier pair is held by fewer than two replies")]
#[case::unknown_wait(script(10, &[r#"{"wait":"forever","then":"close"}"#]), "cannot parse mock script: unknown variant `forever`")]
#[case::oversized(vec![b' '; 16 * 1024 * 1024 + 1], "mock script is 16777217 bytes, past the 16 MiB limit")]
#[case::valid_seed(seed("valid"), "")]
#[case::invalid_seed(
    seed("invalid"),
    "mock script barrier solo is held by fewer than two replies"
)]
fn parser_enforces_every_bound(#[case] bytes: Vec<u8>, #[case] error: &str) {
    let parsed = parse_script(&bytes).map(|_script| String::new());
    assert!(
        parsed
            .unwrap_or_else(|failure| failure.to_string())
            .starts_with(error)
    );
}

#[test]
fn trace_selects_each_reply_and_stops_at_its_bound() {
    let mut recording = Recording::new(parse_script(&script(10, &[OK; 256])).unwrap());
    let request = ProviderRequest {
        method: "POST".to_owned(),
        path: "/".to_owned(),
        headers: BTreeMap::new(),
        body: Value::Null,
    };
    let admitted = iter::repeat_with(|| {
        let ticket = recording.admit(request.clone());
        ticket
            .map(|admission| admission.index)
            .map_err(|error| error.to_string())
    })
    .take(MAX_TRACE + 1)
    .collect::<Vec<_>>();
    let exhausted = Err("provider answer sequence exhausted".to_owned());
    assert_eq!(
        [&admitted[255], &admitted[256], &admitted[MAX_TRACE - 1]],
        [&Ok(255), &exhausted, &exhausted]
    );
    assert_eq!(
        admitted[MAX_TRACE],
        Err("provider trace exceeded 1024 calls".to_owned())
    );
    let trace = recording.finish_run().unwrap();
    let outcomes = (trace.len(), trace[255].outcome, trace[256].outcome);
    assert_eq!(
        outcomes,
        (MAX_TRACE, CallOutcome::Waiting, CallOutcome::Exhausted)
    );
}

#[rstest]
#[case::caller_hangs_up_during_delay(
    r#"{"wait":{"delay":{"millis":1000}},"then":"close"}"#,
    true,
    CallOutcome::Cancelled
)]
#[case::caller_hangs_up_at_barrier(PAIR, true, CallOutcome::Cancelled)]
#[case::barrier_misses_deadline(PAIR, false, CallOutcome::DeadlineExceeded)]
fn waits_end_on_hang_up_or_deadline(
    #[case] reply: &str,
    #[case] hang_up: bool,
    #[case] outcome: CallOutcome,
) {
    let mock = start(1000, &[reply, PAIR, PAIR]);
    let mut stream = connect(&mock, "/w");
    let mut received = String::new();
    if hang_up {
        thread::park_timeout(Duration::from_millis(100));
        drop(stream);
    } else {
        let _read = stream.read_to_string(&mut received);
    }
    let trace = mock.finish().unwrap();
    assert_eq!((trace[0].outcome, received.as_str()), (outcome, ""));
}
