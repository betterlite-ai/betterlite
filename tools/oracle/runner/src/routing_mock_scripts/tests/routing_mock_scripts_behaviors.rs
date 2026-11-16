use std::thread;

use rstest::rstest;

use super::{ANSWERED, BUSY, CallOutcome, OK, PAIR, SLOW, call, start, steps};

const REFUSED: CallOutcome = CallOutcome::Answered { status: 503 };
const CLOSED: CallOutcome = CallOutcome::Closed;

#[rstest]
#[case::retries_then_answer(&[BUSY, BUSY, OK], [&["/a", "/a", "/a"][..], &["/b", "/b"]], [&[("/a", REFUSED), ("/a", REFUSED), ("/a", ANSWERED)][..], &[("/b", REFUSED), ("/b", REFUSED)]], ["503", "503", "200"])]
#[case::changed_order(&[OK, BUSY], [&["/a", "/b"][..], &["/b", "/a"]], [&[("/a", ANSWERED), ("/b", REFUSED)][..], &[("/b", ANSWERED), ("/a", REFUSED)]], ["200", "503", ""])]
#[case::delay_then_close(&[SLOW, r#"{"then":"close"}"#], [&["/a", "/b"][..], &["/a"]], [&[("/a", ANSWERED), ("/b", CLOSED)][..], &[("/a", ANSWERED)]], ["200", "", ""])]
fn each_run_starts_from_the_first_reply(
    #[case] replies: &[&str],
    #[case] runs: [&[&str]; 2],
    #[case] expected: [&[(&str, CallOutcome)]; 2],
    #[case] statuses: [&str; 3],
) {
    let mock = start(1000, replies);
    let seen = runs.map(|paths| {
        let received = paths
            .iter()
            .map(|path| call(&mock, path))
            .collect::<Vec<_>>();
        (received, mock.finish_run().unwrap())
    });
    let [first, second] = &seen;
    assert_eq!([steps(&first.1), steps(&second.1)], expected);
    assert_eq!(first.0, statuses[..first.0.len()]);
    let key = &first.1[0].request.headers["authorization"];
    assert!(key.starts_with("redacted:") && !key.contains("sk-secret"));
    assert_eq!(&second.1[0].request.headers["authorization"], key);
}

#[test]
fn barrier_releases_once_every_party_arrived() {
    let mock = start(5000, &[OK, PAIR, PAIR]);
    let first = call(&mock, "/a");
    let (second, third) = thread::scope(|scope| {
        let waiting = scope.spawn(|| call(&mock, "/b"));
        let arriving = call(&mock, "/b");
        (waiting.join().unwrap(), arriving)
    });
    assert_eq!([first, second, third], ["200", "200", "200"]);
    let trace = mock.finish().unwrap();
    assert_eq!(
        steps(&trace),
        [("/a", ANSWERED), ("/b", ANSWERED), ("/b", ANSWERED)]
    );
}
