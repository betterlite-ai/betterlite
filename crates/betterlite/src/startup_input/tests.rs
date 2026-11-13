#![cfg(test)]

use std::convert::Infallible;
use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::PoisonError;
use std::sync::atomic::{AtomicUsize, Ordering};

use rstest::rstest;

use super::{
    FALLBACK_PORT_HIGH, FALLBACK_PORT_LOW, LocalPortProber, MAX_INPUT_BYTES, PortProber,
    PositionalCommand, StartupInputError, check_metrics_port, choose_port, classify_positional,
    decode_headers, decode_worker_config, xai_login_lines,
};

struct TestRng {
    sample: Option<u32>,
    draws: AtomicUsize,
}

impl rand::TryRng for TestRng {
    type Error = Infallible;

    fn try_next_u32(&mut self) -> Result<u32, Infallible> {
        let _previous = self.draws.fetch_add(1, Ordering::Relaxed);
        Ok(self.sample.take().unwrap_or(0))
    }

    fn try_next_u64(&mut self) -> Result<u64, Infallible> {
        self.try_next_u32().map(u64::from)
    }

    fn try_fill_bytes(&mut self, dst: &mut [u8]) -> Result<(), Infallible> {
        dst.fill(0);
        Ok(())
    }
}

fn test_rng(sample: Option<u32>) -> TestRng {
    TestRng {
        sample,
        draws: AtomicUsize::new(0),
    }
}

fn draw_count(rng: &TestRng) -> usize {
    rng.draws.load(Ordering::Relaxed)
}

struct RecordingProber {
    busy: bool,
    asked: Mutex<Vec<u16>>,
}

impl PortProber for RecordingProber {
    fn in_use(&self, port: u16) -> bool {
        self.asked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(port);
        self.busy
    }
}

fn test_prober(busy: bool) -> RecordingProber {
    RecordingProber {
        busy,
        asked: Mutex::new(Vec::new()),
    }
}

fn asked_ports(prober: &RecordingProber) -> Vec<u16> {
    prober
        .asked
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

#[rstest]
#[case::no_arguments(vec![], PositionalCommand::None)]
#[case::xai_login(
    vec!["xai-oauth".to_owned(), "login".to_owned()],
    PositionalCommand::XaiOauthLogin
)]
fn classify_positional_reads_known_commands(
    #[case] args: Vec<String>,
    #[case] expected: PositionalCommand,
) {
    assert_eq!(classify_positional(&args).unwrap(), expected);
}

#[rstest]
#[case::unknown_words(vec!["foo".to_owned(), "bar".to_owned()], "Unknown command: foo bar")]
#[case::partial_xai(vec!["xai-oauth".to_owned()], "Unknown command: xai-oauth")]
#[case::extra_word(
    vec!["xai-oauth".to_owned(), "login".to_owned(), "now".to_owned()],
    "Unknown command: xai-oauth login now"
)]
#[case::wrong_second_word(
    vec!["xai-oauth".to_owned(), "logout".to_owned()],
    "Unknown command: xai-oauth logout"
)]
fn classify_positional_rejects_other_arguments(#[case] args: Vec<String>, #[case] expected: &str) {
    let error = classify_positional(&args).expect_err("the arguments are not a command");

    assert!(matches!(error, StartupInputError::UnknownCommand { .. }));
    assert_eq!(error.to_string(), expected);
}

#[test]
fn xai_login_lines_name_the_saved_file() {
    let lines = xai_login_lines(Path::new("/home/user/.config/xai/auth.json"), None);

    assert_eq!(
        lines,
        ["xAI OAuth login successful. Credentials saved to /home/user/.config/xai/auth.json."]
    );
}

#[test]
fn xai_login_lines_add_the_expiry_when_present() {
    let lines = xai_login_lines(
        Path::new("/home/user/.config/xai/auth.json"),
        Some(1_700_000_000),
    );

    assert_eq!(
        lines,
        [
            "xAI OAuth login successful. Credentials saved to /home/user/.config/xai/auth.json.",
            "Access token expires at 1700000000."
        ]
    );
}

#[test]
fn decode_headers_returns_none_for_an_empty_value() {
    assert_eq!(decode_headers("").unwrap(), None);
}

#[test]
fn decode_headers_reads_a_json_object_of_strings() {
    let headers = decode_headers(r#"{"x-api-key":"sk-1","anthropic-version":"2023-06-01"}"#)
        .unwrap()
        .expect("headers are present");

    assert_eq!(headers.get("x-api-key").map(String::as_str), Some("sk-1"));
    assert_eq!(
        headers.get("anthropic-version").map(String::as_str),
        Some("2023-06-01")
    );
    assert_eq!(headers.len(), 2);
}

#[test]
fn decode_headers_keeps_the_last_duplicate_key() {
    let headers = decode_headers(r#"{"a":"first","a":"last"}"#)
        .unwrap()
        .expect("headers are present");

    assert_eq!(headers.get("a").map(String::as_str), Some("last"));
}

#[rstest]
#[case::non_object("[]")]
#[case::top_level_string(r#""sk-secret-token""#)]
#[case::non_string_value(r#"{"Authorization":98765432}"#)]
#[case::boolean_value(r#"{"Authorization":true}"#)]
#[case::nested_value(r#"{"a":{"b":"sk-secret-token"}}"#)]
fn decode_headers_rejects_what_is_not_a_string_object(#[case] raw: &str) {
    let error = decode_headers(raw).expect_err("the value is rejected");

    assert!(matches!(error, StartupInputError::HeadersShape), "{error}");
    assert!(!error.to_string().contains("sk-secret-token"), "{error}");
    assert!(!error.to_string().contains("98765432"), "{error}");
}

#[rstest]
#[case::truncated("{", "unexpected end of input")]
#[case::bad_token(r#"{"a":sk-secret-token}"#, "syntax error")]
fn decode_headers_reports_invalid_json_without_the_input(
    #[case] raw: &str,
    #[case] category: &str,
) {
    let error = decode_headers(raw).expect_err("the text is rejected");

    assert!(
        matches!(error, StartupInputError::HeadersSyntax { .. }),
        "{error}"
    );
    assert!(error.to_string().contains(category), "{error}");
    assert!(!error.to_string().contains("sk-secret-token"), "{error}");
}

#[test]
fn decode_headers_accepts_the_size_limit_and_rejects_one_byte_more() {
    let at_limit = format!(
        r#"{{"a":"{}"}}"#,
        "a".repeat(MAX_INPUT_BYTES.saturating_sub(8))
    );
    let over_limit = format!(
        r#"{{"a":"{}"}}"#,
        "a".repeat(MAX_INPUT_BYTES.saturating_sub(7))
    );

    assert_eq!(at_limit.len(), MAX_INPUT_BYTES);
    let headers = decode_headers(&at_limit)
        .unwrap()
        .expect("headers are present");
    assert_eq!(headers.len(), 1);
    assert_eq!(over_limit.len(), MAX_INPUT_BYTES.saturating_add(1));
    assert!(matches!(
        decode_headers(&over_limit),
        Err(StartupInputError::HeadersTooLong { .. })
    ));
}

#[test]
fn decode_worker_config_reads_both_flags() {
    let flags = decode_worker_config(r#"{"debug":true,"detailed_debug":false}"#).unwrap();

    assert_eq!(flags.debug, Some(true));
    assert_eq!(flags.detailed_debug, Some(false));
}

#[test]
fn decode_worker_config_leaves_absent_flags_unset_and_ignores_other_keys() {
    let flags =
        decode_worker_config(r#"{"model":"gpt-4","nested":{"debug":true},"debug":null}"#).unwrap();

    assert_eq!(flags.debug, None);
    assert_eq!(flags.detailed_debug, None);
}

#[test]
fn decode_worker_config_reads_null_as_unset() {
    let flags = decode_worker_config(r#"{"debug":null,"detailed_debug":true}"#).unwrap();

    assert_eq!(flags.debug, None);
    assert_eq!(flags.detailed_debug, Some(true));
}

#[test]
fn decode_worker_config_keeps_the_last_duplicate_key() {
    let flags = decode_worker_config(r#"{"debug":false,"debug":true}"#).unwrap();

    assert_eq!(flags.debug, Some(true));
}

#[rstest]
#[case::empty_array("[]")]
#[case::true_pair("[true,true]")]
#[case::false_pair("[false,false]")]
#[case::number("5")]
#[case::string(r#""debug""#)]
fn decode_worker_config_rejects_what_is_not_an_object(#[case] raw: &str) {
    let error = decode_worker_config(raw).expect_err("the value is rejected");

    assert!(
        matches!(error, StartupInputError::WorkerConfigShape),
        "{error}"
    );
}

#[test]
fn decode_worker_config_rejects_invalid_json() {
    let error = decode_worker_config("{").expect_err("the text is rejected");

    assert!(
        matches!(error, StartupInputError::WorkerConfigSyntax { .. }),
        "{error}"
    );
}

#[rstest]
#[case::string_flag(r#"{"debug":"true"}"#, "debug")]
#[case::number_flag(r#"{"detailed_debug":1}"#, "detailed_debug")]
fn decode_worker_config_rejects_non_boolean_flags(#[case] raw: &str, #[case] key: &str) {
    let error = decode_worker_config(raw).expect_err("the value is rejected");

    assert!(
        matches!(error, StartupInputError::WorkerConfigFlag { .. }),
        "{error}"
    );
    assert!(error.to_string().contains(key), "{error}");
}

#[test]
fn decode_worker_config_accepts_the_size_limit_and_rejects_one_byte_more() {
    let at_limit = format!(
        r#"{{"debug":true,"x":"{}"}}"#,
        "a".repeat(MAX_INPUT_BYTES.saturating_sub(21))
    );
    let over_limit = format!(
        r#"{{"debug":true,"x":"{}"}}"#,
        "a".repeat(MAX_INPUT_BYTES.saturating_sub(20))
    );

    assert_eq!(at_limit.len(), MAX_INPUT_BYTES);
    let flags = decode_worker_config(&at_limit).unwrap();
    assert_eq!(flags.debug, Some(true));
    assert_eq!(over_limit.len(), MAX_INPUT_BYTES.saturating_add(1));
    assert!(matches!(
        decode_worker_config(&over_limit),
        Err(StartupInputError::WorkerConfigTooLong { .. })
    ));
}

#[test]
fn check_metrics_port_rejects_the_same_port() {
    let error = check_metrics_port(Some(4_000), 4_000).expect_err("the ports clash");

    assert!(matches!(error, StartupInputError::MetricsPortClash));
    assert_eq!(
        error.to_string(),
        "--prometheus_metrics_port must differ from --port"
    );
}

#[rstest]
#[case::absent(None, 4_000)]
#[case::different(Some(8_000), 4_000)]
fn check_metrics_port_accepts_other_ports(#[case] metrics: Option<u16>, #[case] port: u16) {
    check_metrics_port(metrics, port).unwrap();
}

#[test]
fn choose_port_keeps_a_port_that_is_not_the_default() {
    let prober = test_prober(true);
    let mut rng = test_rng(None);

    assert_eq!(choose_port(8_080, &prober, &mut rng), 8_080);
    assert!(asked_ports(&prober).is_empty());
    assert_eq!(draw_count(&rng), 0);
}

#[test]
fn choose_port_keeps_the_default_port_when_it_is_free() {
    let prober = test_prober(false);
    let mut rng = test_rng(None);

    assert_eq!(choose_port(4_000, &prober, &mut rng), 4_000);
    assert_eq!(asked_ports(&prober), [4_000]);
    assert_eq!(draw_count(&rng), 0);
}

#[test]
fn choose_port_draws_a_fallback_for_a_busy_default_port() {
    let prober = test_prober(true);
    let mut rng = test_rng(Some(2_675_017_966));

    assert_eq!(choose_port(4_000, &prober, &mut rng), 31_000);
    assert_eq!(asked_ports(&prober), [4_000]);
    assert!(draw_count(&rng) > 0);
}

#[test]
fn choose_port_draws_the_fallback_range_inclusively() {
    let prober = test_prober(true);
    let mut low_rng = test_rng(Some(0));
    let mut high_rng = test_rng(Some(u32::MAX));

    assert_eq!(choose_port(4_000, &prober, &mut low_rng), FALLBACK_PORT_LOW);
    assert_eq!(
        choose_port(4_000, &prober, &mut high_rng),
        FALLBACK_PORT_HIGH
    );
    assert_eq!(FALLBACK_PORT_LOW, 1_024);
    assert_eq!(FALLBACK_PORT_HIGH, 49_152);
}

#[test]
fn local_port_prober_finds_a_listening_ipv4_port() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();

    assert!(LocalPortProber.in_use(port));
}

#[test]
fn local_port_prober_answers_false_when_nothing_listens() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);

    assert!(!LocalPortProber.in_use(port));
}

#[test]
fn local_port_prober_does_not_try_the_ipv6_loopback() {
    let Ok(listener) = TcpListener::bind("[::1]:0") else {
        return;
    };
    let port = listener.local_addr().unwrap().port();

    assert!(!LocalPortProber.in_use(port));
}

#[rstest]
fn replays_startup_input_json_corpus(#[files("fuzz/corpus/startup_input_json/*")] path: PathBuf) {
    let bytes = fs::read(&path).unwrap();
    let text = String::from_utf8(bytes).unwrap_or_else(|error| {
        error
            .as_bytes()
            .iter()
            .map(|byte| char::from(*byte))
            .collect()
    });

    drop(decode_headers(&text));
    drop(decode_worker_config(&text));
}
