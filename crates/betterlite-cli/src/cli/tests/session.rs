use rstest::rstest;

use crate::cli::session::{CliClock, SystemEntropy};
use crate::cli::tests::support::{Captured, FakeEnvironment, FakeVault, proxy_context, session};
use crate::pkce_login::{Entropy, WallClock};

const LATE_2023: i64 = 1_700_000_000;

#[rstest]
#[case::default(None, 24)]
#[case::first_name(Some(("CLI_JWT_EXPIRATION_HOURS", "5")), 5)]
#[case::second_name(Some(("LITELLM_CLI_JWT_EXPIRATION_HOURS", "7")), 7)]
#[case::empty_counts_as_unset(Some(("CLI_JWT_EXPIRATION_HOURS", "")), 24)]
fn the_session_reads_the_expiration_hours_from_the_environment(
    #[case] variable: Option<(&str, &str)>,
    #[case] expected: i64,
) {
    let mut environment = FakeEnvironment::default();
    if let Some((name, value)) = variable {
        environment = environment.with_variable(name, value);
    }
    let mut vault = FakeVault;
    let out = Captured::default();
    let err = Captured::default();
    let context = proxy_context("http://proxy.example");
    let session = session(&context, &environment, &mut vault, &out, &err);
    assert_eq!(session.expiration_hours, expected);
}

#[test]
fn the_wall_clock_reports_the_current_time() {
    let seconds = WallClock::now_seconds(&CliClock);
    assert!(seconds > LATE_2023, "{seconds} is not the current time");
}

#[test]
fn the_entropy_source_fills_the_buffer() {
    let mut bytes = [0_u8; 32];
    Entropy::fill(&mut SystemEntropy, &mut bytes).unwrap();
    assert!(bytes.iter().any(|byte| *byte != 0));
}
