#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::str;
use std::sync::Arc;

use betterlite_config::sections::settings::JsonValue;
use betterlite_logging::private_json::files::PrivateFiles;
use rstest::rstest;
use secrecy::SecretString;
use tempfile::TempDir;

use crate::claude_settings::SettingsDocument;
use crate::claude_settings::choices::STARTING_MODEL_ROLE;
use crate::cli_context::CliContext;
use crate::credentials::notice::{
    KEYRING_INSTALL_HINT, STRANDED_CREDENTIAL_MESSAGE, UNCHECKED_KEYCHAIN_MESSAGE,
};
use crate::keyring::KeyringUnusable;
use crate::token_records::secret::decode_secret;

use super::auth_command::{
    AUTH_ABOUT, AuthCommand, PRINT_TOKEN_ABOUT, PRINT_TOKEN_LONG_ABOUT, run_auth_command,
};
use super::configure::{ConfigureClaudeRequest, configure_claude_code};
use super::error::SessionError;
use super::logout::{LOGOUT_ABOUT, LogoutRequest, logout};
use super::print_token::{PrintTokenRequest, print_token};
use super::whoami::{WHOAMI_ABOUT, WhoamiRequest, whoami};

mod support;

use support::{
    FakeEnvironment, FakeFiles, FakeHttp, FakeVault, FixedWallClock, OTHER_SERVER,
    REVOCATION_ENDPOINT, RecordedRequest, Refusal, Reply, SERVER, TOKEN_ENDPOINT, context, exposed,
    form_fields, record, response, secret, token_response,
};

fn text(bytes: &[u8]) -> &str {
    str::from_utf8(bytes).expect("the captured output is UTF-8")
}

fn token_path() -> &'static Path {
    Path::new("/home/tester/.litellm/token.json")
}

#[test]
fn logout_help_text_matches_litellm() {
    assert_eq!(LOGOUT_ABOUT, "Logout and clear stored authentication");
}

#[test]
fn logout_revokes_the_stored_credential_before_clearing() {
    let files = FakeFiles::holding(record());
    let mut vault = FakeVault::unreachable_read();
    let http = Arc::new(FakeHttp::new(vec![Reply::Answer(response(200, "{}"))]));
    let mut stdout = Vec::new();
    let mut request = LogoutRequest {
        files: &files,
        vault: &mut vault,
        http: Arc::clone(&http),
        token_path: token_path(),
        stdout: &mut stdout,
    };
    logout(&mut request).expect("logs out");
    let requests = http.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].url, REVOCATION_ENDPOINT);
    assert_eq!(
        form_fields(&requests[0].body),
        vec![
            ("token".to_owned(), "refresh-secret".to_owned()),
            ("token_type_hint".to_owned(), "refresh_token".to_owned()),
            ("client_id".to_owned(), "client-1".to_owned()),
        ]
    );
    assert_eq!(files.removals(), 1);
    assert_eq!(vault.erases, 1);
    assert_eq!(
        text(&stdout),
        "Logged out successfully. Authentication token cleared.\n"
    );
}

#[test]
fn logout_sends_no_request_without_a_stored_credential() {
    let files = FakeFiles::empty();
    let mut vault = FakeVault::unreachable_read();
    let http = Arc::new(FakeHttp::new(Vec::new()));
    let mut stdout = Vec::new();
    let mut request = LogoutRequest {
        files: &files,
        vault: &mut vault,
        http: Arc::clone(&http),
        token_path: token_path(),
        stdout: &mut stdout,
    };
    logout(&mut request).expect("logs out");
    assert!(http.requests().is_empty());
    assert_eq!(files.removals(), 1);
    assert_eq!(vault.erases, 1);
    assert_eq!(
        text(&stdout),
        "Logged out successfully. Authentication token cleared.\n"
    );
}

#[test]
fn logout_sends_no_request_for_a_credential_without_a_refresh_token() {
    let mut stored = record();
    stored.refresh_token = None;
    let files = FakeFiles::holding(stored);
    let mut vault = FakeVault::unreachable_read();
    let http = Arc::new(FakeHttp::new(Vec::new()));
    let mut stdout = Vec::new();
    let mut request = LogoutRequest {
        files: &files,
        vault: &mut vault,
        http: Arc::clone(&http),
        token_path: token_path(),
        stdout: &mut stdout,
    };
    logout(&mut request).expect("logs out");
    assert!(http.requests().is_empty());
    assert_eq!(files.removals(), 1);
}

#[test]
fn logout_stops_without_clearing_when_revocation_is_unavailable() {
    let files = FakeFiles::holding(record());
    let mut vault = FakeVault::unreachable_read();
    let http = Arc::new(FakeHttp::new(vec![Reply::Answer(response(503, "busy"))]));
    let mut stdout = Vec::new();
    let mut request = LogoutRequest {
        files: &files,
        vault: &mut vault,
        http: Arc::clone(&http),
        token_path: token_path(),
        stdout: &mut stdout,
    };
    let error = logout(&mut request).expect_err("stops");
    assert!(matches!(
        &error,
        SessionError::RevocationUnavailable { reason } if reason.as_str() == "busy"
    ));
    assert_eq!(
        error.to_string(),
        "the proxy could not record the revocation (busy), so nothing was cleared; run `lite logout` again shortly"
    );
    assert_eq!(files.removals(), 0);
    assert_eq!(vault.erases, 0);
    assert!(stdout.is_empty());
}

#[test]
fn logout_reports_a_failed_revocation_and_still_clears() {
    let files = FakeFiles::holding(record());
    let mut vault = FakeVault::unreachable_read();
    let http = Arc::new(FakeHttp::new(vec![Reply::Answer(response(
        400,
        r#"{"error_description":"denied"}"#,
    ))]));
    let mut stdout = Vec::new();
    let mut request = LogoutRequest {
        files: &files,
        vault: &mut vault,
        http: Arc::clone(&http),
        token_path: token_path(),
        stdout: &mut stdout,
    };
    logout(&mut request).expect("logs out");
    assert_eq!(files.removals(), 1);
    assert_eq!(
        text(&stdout),
        "Could not revoke the refresh token on the proxy (denied); it expires on its own.\n\
         Logged out successfully. Authentication token cleared.\n"
    );
}

#[test]
fn logout_prints_the_not_cleared_lines() {
    let mut files = FakeFiles::holding(record());
    files
        .refusals
        .extend([Refusal::Stage, Refusal::Scrub, Refusal::Remove]);
    let mut vault = FakeVault::unreachable_read();
    let http = Arc::new(FakeHttp::new(vec![Reply::Answer(response(200, "{}"))]));
    let mut stdout = Vec::new();
    let mut request = LogoutRequest {
        files: &files,
        vault: &mut vault,
        http: Arc::clone(&http),
        token_path: token_path(),
        stdout: &mut stdout,
    };
    logout(&mut request).expect("logs out");
    assert_eq!(
        text(&stdout),
        "Your credential is still in /home/tester/.litellm/token.json, which could not be removed: the fixture refuses the removal.\n\
         Delete that file, or make the directory writable and run 'lite logout' again.\n"
    );
}

#[test]
fn logout_prints_the_stranded_lines() {
    let files = FakeFiles::holding(record());
    let mut vault = FakeVault::unreachable_read();
    vault.erase_result = support::EraseResult::Stranded;
    let http = Arc::new(FakeHttp::new(vec![Reply::Answer(response(200, "{}"))]));
    let mut stdout = Vec::new();
    let mut request = LogoutRequest {
        files: &files,
        vault: &mut vault,
        http: Arc::clone(&http),
        token_path: token_path(),
        stdout: &mut stdout,
    };
    logout(&mut request).expect("logs out");
    assert_eq!(
        text(&stdout),
        format!(
            "{STRANDED_CREDENTIAL_MESSAGE}\nUnlock your keychain and run 'lite logout' again to clear it.\n"
        )
    );
}

#[rstest]
#[case::keyring_not_installed(
    KeyringUnusable::KeyringNotInstalled,
    "Install the keyring package with: pip install 'litellm[cli]', then run 'lite logout' again."
)]
#[case::keyring_disabled(
    KeyringUnusable::KeyringDisabled,
    "Unset LITELLM_CLI_DISABLE_KEYRING and run 'lite logout' again to clear it."
)]
#[case::keyring_unreachable(
    KeyringUnusable::KeyringUnreachable,
    "Unlock your keychain and run 'lite logout' again to clear it."
)]
fn logout_prints_the_unchecked_keychain_lines(
    #[case] failure: KeyringUnusable,
    #[case] second: &str,
) {
    let mut files = FakeFiles::holding(record());
    files.refusals.extend([Refusal::Stage, Refusal::Scrub]);
    let mut vault = FakeVault::unreachable_read();
    vault.erase_result = support::EraseResult::Unavailable(failure);
    let http = Arc::new(FakeHttp::new(vec![Reply::Answer(response(200, "{}"))]));
    let mut stdout = Vec::new();
    let mut request = LogoutRequest {
        files: &files,
        vault: &mut vault,
        http: Arc::clone(&http),
        token_path: token_path(),
        stdout: &mut stdout,
    };
    logout(&mut request).expect("logs out");
    assert_eq!(
        text(&stdout),
        format!("{UNCHECKED_KEYCHAIN_MESSAGE}\n{second}\n")
    );
    assert_eq!(files.removals(), 1);
}

struct PrintFixture {
    context: CliContext,
    files: FakeFiles,
    vault: FakeVault,
    http: Arc<FakeHttp>,
    expiration_hours: i64,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

impl PrintFixture {
    fn new(files: FakeFiles, replies: Vec<Reply>) -> Self {
        Self {
            context: context(SERVER),
            files,
            vault: FakeVault::unreachable_read(),
            http: Arc::new(FakeHttp::new(replies)),
            expiration_hours: 24,
            stdout: Vec::new(),
            stderr: Vec::new(),
        }
    }

    fn run(&mut self, now: i64) -> Result<(), SessionError> {
        let mut request = PrintTokenRequest {
            context: &self.context,
            files: &self.files,
            vault: &mut self.vault,
            http: Arc::clone(&self.http),
            clock: Arc::new(FixedWallClock(now)),
            expiration_hours: self.expiration_hours,
            token_path: token_path(),
            stdout: &mut self.stdout,
            stderr: &mut self.stderr,
        };
        print_token(&mut request)
    }

    fn stdout(&self) -> &str {
        text(&self.stdout)
    }

    fn stderr(&self) -> &str {
        text(&self.stderr)
    }

    fn requests(&self) -> Vec<RecordedRequest> {
        self.http.requests()
    }
}

#[test]
fn print_token_help_texts_match_litellm() {
    assert_eq!(
        AUTH_ABOUT,
        "Manage CLI authentication (apiKeyHelper support, etc.)"
    );
    assert_eq!(PRINT_TOKEN_ABOUT, "Print a valid API token for this proxy.");
    assert_eq!(
        PRINT_TOKEN_LONG_ABOUT,
        "Print a valid API token for this proxy.\n\nDesigned to be used as Claude Code's `apiKeyHelper` (https://docs.claude.com/en/docs/claude-code/settings): stdout must contain only the token, so all diagnostics go to stderr. The token expires after `LITELLM_CLI_JWT_EXPIRATION_HOURS` (default 24h); a `lite login --pkce` token renews itself here first, and once a token has expired for good, run the same `lite login` command again."
    );
}

#[test]
fn print_token_auth_command_runs_the_print_token_command() {
    let mut fixture = PrintFixture::new(FakeFiles::holding(record()), Vec::new());
    let mut request = PrintTokenRequest {
        context: &fixture.context,
        files: &fixture.files,
        vault: &mut fixture.vault,
        http: Arc::clone(&fixture.http),
        clock: Arc::new(FixedWallClock(1_000)),
        expiration_hours: 24,
        token_path: token_path(),
        stdout: &mut fixture.stdout,
        stderr: &mut fixture.stderr,
    };
    run_auth_command(AuthCommand::PrintToken, &mut request).expect("prints the key");
    assert_eq!(fixture.stdout(), "sk-secret\n");
}

#[test]
fn print_token_writes_only_the_key_and_a_newline() {
    let mut stored = record();
    stored.refresh_token = None;
    let mut fixture = PrintFixture::new(FakeFiles::holding(stored), Vec::new());
    fixture.run(1_000).expect("prints the key");
    assert_eq!(fixture.stdout(), "sk-secret\n");
    assert!(fixture.stderr().is_empty());
    assert!(fixture.requests().is_empty());
}

#[test]
fn print_token_takes_the_context_key_when_the_file_provided_it() {
    let mut fixture = PrintFixture::new(FakeFiles::holding(record()), Vec::new());
    fixture.context.api_key = Some(secret("sk-context"));
    fixture.context.api_key_from_token_file = true;
    fixture.run(1_000).expect("prints the context key");
    assert_eq!(fixture.stdout(), "sk-context\n");
    assert!(fixture.stderr().is_empty());
    assert!(fixture.requests().is_empty());
}

#[test]
fn print_token_ignores_the_context_key_for_another_stored_server() {
    let mut fixture = PrintFixture::new(FakeFiles::holding(record()), Vec::new());
    fixture.context = context(OTHER_SERVER);
    fixture.context.api_key = Some(secret("sk-context"));
    fixture.context.api_key_from_token_file = true;
    fixture.run(1_000).expect("prints the stored key");
    assert_eq!(fixture.stdout(), "sk-secret\n");
    assert!(fixture.stderr().is_empty());
}

#[test]
fn print_token_rejects_another_server_when_the_base_url_was_explicit() {
    let mut fixture = PrintFixture::new(FakeFiles::holding(record()), Vec::new());
    fixture.context = context(OTHER_SERVER);
    fixture.context.base_url_explicit = true;
    let error = fixture.run(1_000).expect_err("rejects the server");
    assert!(matches!(error, SessionError::NotAuthenticatedForServer));
    assert!(error.to_string().contains("lite login"));
    assert!(fixture.stdout().is_empty());
    assert!(fixture.stderr().is_empty());
}

#[test]
fn print_token_rejects_a_missing_credential() {
    let mut fixture = PrintFixture::new(FakeFiles::empty(), Vec::new());
    let error = fixture.run(1_000).expect_err("rejects");
    assert!(matches!(error, SessionError::NotAuthenticated));
    assert!(error.to_string().contains("lite login"));
    assert!(fixture.stdout().is_empty());
}

#[test]
fn print_token_rejects_a_stale_credential_without_renewal() {
    let mut stored = record();
    stored.expires_at = Some(900.0_f64);
    stored.refresh_token = None;
    let mut fixture = PrintFixture::new(FakeFiles::holding(stored), Vec::new());
    let error = fixture.run(1_000).expect_err("rejects");
    assert!(matches!(error, SessionError::TokenExpired));
    assert!(error.to_string().contains("lite login"));
    assert!(fixture.stdout().is_empty());
}

#[test]
fn print_token_accepts_an_explicit_base_url_for_this_server() {
    let files = FakeFiles::holding(record());
    let mut fixture = PrintFixture::new(files, Vec::new());
    fixture.context.base_url_explicit = true;
    fixture.run(1_000).expect("prints the stored key");
    assert_eq!(fixture.stdout(), "sk-secret\n");
    assert!(fixture.stderr().is_empty());
}

#[test]
fn print_token_checks_the_server_before_freshness() {
    let mut stored = record();
    stored.expires_at = Some(0.0_f64);
    stored.refresh_token = None;
    let mut fixture = PrintFixture::new(FakeFiles::holding(stored), Vec::new());
    fixture.context = context(OTHER_SERVER);
    fixture.context.base_url_explicit = true;
    let error = fixture.run(1_000).expect_err("rejects");
    assert!(matches!(error, SessionError::NotAuthenticatedForServer));
}

#[test]
fn print_token_checks_freshness_before_the_keychain() {
    let mut stored = record();
    stored.key = None;
    stored.expires_at = Some(0.0_f64);
    stored.refresh_token = None;
    let mut fixture = PrintFixture::new(FakeFiles::holding(stored), Vec::new());
    let error = fixture.run(1_000).expect_err("rejects");
    assert!(matches!(error, SessionError::TokenExpired));
}

#[test]
fn print_token_renews_a_stale_credential_with_a_refresh_token() {
    let mut stored = record();
    stored.expires_at = Some(900.0_f64);
    let mut fixture = PrintFixture::new(
        FakeFiles::holding(stored),
        vec![Reply::Answer(token_response(
            "sk-renewed",
            "refresh-2",
            3_600,
        ))],
    );
    fixture.run(1_000).expect("renews");
    assert_eq!(fixture.stdout(), "sk-renewed\n");
    assert!(fixture.stderr().is_empty());
    let requests = fixture.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].url, TOKEN_ENDPOINT);
    assert_eq!(
        form_fields(&requests[0].body),
        vec![
            ("grant_type".to_owned(), "refresh_token".to_owned()),
            ("refresh_token".to_owned(), "refresh-secret".to_owned()),
            ("client_id".to_owned(), "client-1".to_owned()),
            (
                "resource".to_owned(),
                "https://gateway.example/v1".to_owned()
            ),
        ]
    );
    assert_saved_renewal(&fixture.files, &fixture.vault);
}

fn assert_saved_renewal(files: &FakeFiles, vault: &FakeVault) {
    let committed = files.committed();
    let renewed = committed.last().expect("saves the renewed credential");
    assert_eq!(renewed.expires_at, Some(4_600.0_f64));
    let blob = vault.writes.last().expect("stores the renewed secret");
    let decoded = decode_secret(exposed(blob), SERVER).expect("decodes the renewed secret");
    assert_eq!(decoded.key.as_ref().map(exposed), Some("sk-renewed"));
    assert_eq!(
        decoded.refresh_token.as_ref().map(exposed),
        Some("refresh-2")
    );
}

#[rstest]
#[case::renewal_failure(Reply::Answer(response(503, "busy")), "Could not renew the key: busy")]
#[case::transport_failure(Reply::Transport, "Could not renew the key: the transport failed")]
fn print_token_warns_on_stderr_when_renewal_fails(#[case] reply: Reply, #[case] warning: &str) {
    let mut stored = record();
    stored.expires_at = Some(900.0_f64);
    let mut fixture = PrintFixture::new(FakeFiles::holding(stored), vec![reply]);
    let error = fixture.run(1_000).expect_err("rejects");
    assert!(
        matches!(error, SessionError::UnusableKey { login_command } if login_command == "lite login --pkce")
    );
    assert!(fixture.stdout().is_empty());
    assert_eq!(fixture.stderr(), format!("{warning}\n"));
}

#[test]
fn print_token_reports_an_unreadable_keychain() {
    let mut stored = record();
    stored.key = None;
    let mut fixture = PrintFixture::new(FakeFiles::holding(stored), Vec::new());
    fixture.vault.read_failure = Some(KeyringUnusable::KeyringNotInstalled);
    let error = fixture.run(1_000).expect_err("rejects");
    let SessionError::KeychainUnreadable { notice } = error else {
        panic!("expected the keychain notice");
    };
    assert_eq!(
        notice,
        format!(
            "Your credential is in your OS keychain, which this install cannot read without the keyring package. Install it with: {KEYRING_INSTALL_HINT}, or run 'lite login' to start over."
        )
    );
    assert!(fixture.stdout().is_empty());
}

#[rstest]
#[case::empty_stored_key(None, "lite login")]
#[case::expired_renewable_key(Some(0.0_f64), "lite login --pkce")]
fn print_token_names_the_login_that_fixes_an_unusable_key(
    #[case] expires_at: Option<f64>,
    #[case] command: &str,
) {
    let mut stored = record();
    stored.key = Some(secret(""));
    stored.expires_at = expires_at;
    if expires_at.is_none() {
        stored.refresh_token = None;
    }
    let mut fixture = PrintFixture::new(FakeFiles::holding(stored), Vec::new());
    let error = fixture.run(1_000).expect_err("rejects");
    assert!(
        matches!(error, SessionError::UnusableKey { login_command } if login_command == command),
        "unexpected error: {error}"
    );
    assert!(fixture.stdout().is_empty());
}

#[test]
fn print_token_rejects_a_missing_context_key_for_this_server() {
    let mut fixture = PrintFixture::new(FakeFiles::holding(record()), Vec::new());
    fixture.context.api_key_from_token_file = true;
    let error = fixture.run(1_000).expect_err("rejects");
    assert!(
        matches!(error, SessionError::UnusableKey { login_command } if login_command == "lite login --pkce")
    );
    assert!(fixture.stdout().is_empty());
}

#[test]
fn whoami_help_text_matches_litellm() {
    assert_eq!(WHOAMI_ABOUT, "Show current authentication status");
}

#[test]
fn whoami_reports_when_nothing_is_stored() {
    let files = FakeFiles::empty();
    let mut vault = FakeVault::unreachable_read();
    let mut stdout = Vec::new();
    let mut request = WhoamiRequest {
        files: &files,
        vault: &mut vault,
        clock: Arc::new(FixedWallClock(1_000)),
        expiration_hours: 24,
        stdout: &mut stdout,
    };
    whoami(&mut request).expect("reports");
    assert_eq!(
        text(&stdout),
        "Not authenticated. Run 'lite login' to authenticate.\n"
    );
}

#[test]
fn whoami_prints_the_identity_with_unknown_placeholders() {
    let mut stored = record();
    stored.user_id = String::new();
    stored.user_email = String::new();
    stored.user_role = String::new();
    let files = FakeFiles::holding(stored);
    let mut vault = FakeVault::unreachable_read();
    let mut stdout = Vec::new();
    let mut request = WhoamiRequest {
        files: &files,
        vault: &mut vault,
        clock: Arc::new(FixedWallClock(1_000)),
        expiration_hours: 24,
        stdout: &mut stdout,
    };
    whoami(&mut request).expect("reports");
    assert_eq!(
        text(&stdout),
        "Authenticated\n\
         User Email: Unknown\n\
         User ID: Unknown\n\
         User Role: Unknown\n\
         Token age: 0.0 hours\n"
    );
}

#[rstest]
#[case::set(Some("team-1"), true)]
#[case::empty(Some(""), false)]
#[case::absent(None, false)]
fn whoami_prints_the_team_only_when_set(#[case] team_id: Option<&str>, #[case] printed: bool) {
    let mut stored = record();
    stored.team_id = team_id.map(str::to_owned);
    let files = FakeFiles::holding(stored);
    let mut vault = FakeVault::unreachable_read();
    let mut stdout = Vec::new();
    let mut request = WhoamiRequest {
        files: &files,
        vault: &mut vault,
        clock: Arc::new(FixedWallClock(1_000)),
        expiration_hours: 24,
        stdout: &mut stdout,
    };
    whoami(&mut request).expect("reports");
    assert_eq!(text(&stdout).contains("Team ID: team-1\n"), printed);
}

#[test]
fn whoami_prints_a_keychain_notice_when_the_key_is_missing() {
    let mut stored = record();
    stored.key = None;
    let files = FakeFiles::holding(stored);
    let mut vault = FakeVault::unreachable_read();
    vault.read_failure = Some(KeyringUnusable::KeyringUnreachable);
    let mut stdout = Vec::new();
    let mut request = WhoamiRequest {
        files: &files,
        vault: &mut vault,
        clock: Arc::new(FixedWallClock(1_000)),
        expiration_hours: 24,
        stdout: &mut stdout,
    };
    whoami(&mut request).expect("reports");
    assert_eq!(
        text(&stdout),
        "Signed in, but the credential cannot be read\n\
         User Email: user@example.com\n\
         User ID: user-1\n\
         User Role: cli\n\
         Token age: 0.0 hours\n\
         Your credential is in your OS keychain, which could not be read. Unlock it, or run 'lite login' to start over.\n"
    );
}

#[test]
fn whoami_prints_the_expired_line_without_a_final_period() {
    let mut stored = record();
    stored.expires_at = Some(1_000.0_f64);
    stored.refresh_token = None;
    let files = FakeFiles::holding(stored);
    let mut vault = FakeVault::unreachable_read();
    let mut stdout = Vec::new();
    let mut request = WhoamiRequest {
        files: &files,
        vault: &mut vault,
        clock: Arc::new(FixedWallClock(1_000)),
        expiration_hours: 24,
        stdout: &mut stdout,
    };
    whoami(&mut request).expect("reports");
    assert!(text(&stdout).ends_with("Key expired. Run 'lite login' again\n"));
}

#[test]
fn whoami_names_the_pkce_login_for_an_expired_renewable_credential() {
    let mut stored = record();
    stored.expires_at = Some(999.0_f64);
    let files = FakeFiles::holding(stored);
    let mut vault = FakeVault::unreachable_read();
    let mut stdout = Vec::new();
    let mut request = WhoamiRequest {
        files: &files,
        vault: &mut vault,
        clock: Arc::new(FixedWallClock(1_000)),
        expiration_hours: 24,
        stdout: &mut stdout,
    };
    whoami(&mut request).expect("reports");
    assert!(text(&stdout).ends_with("Key expired. Run 'lite login --pkce' again\n"));
}

#[test]
fn whoami_prints_the_remaining_hours() {
    let mut stored = record();
    stored.expires_at = Some(8_200.0_f64);
    stored.refresh_token = None;
    let files = FakeFiles::holding(stored);
    let mut vault = FakeVault::unreachable_read();
    let mut stdout = Vec::new();
    let mut request = WhoamiRequest {
        files: &files,
        vault: &mut vault,
        clock: Arc::new(FixedWallClock(1_000)),
        expiration_hours: 24,
        stdout: &mut stdout,
    };
    whoami(&mut request).expect("reports");
    assert!(text(&stdout).ends_with("Key expires in: 2.0 hours\n"));
}

#[test]
fn whoami_appends_renewed_on_next_use() {
    let mut stored = record();
    stored.expires_at = Some(8_200.0_f64);
    let files = FakeFiles::holding(stored);
    let mut vault = FakeVault::unreachable_read();
    let mut stdout = Vec::new();
    let mut request = WhoamiRequest {
        files: &files,
        vault: &mut vault,
        clock: Arc::new(FixedWallClock(1_000)),
        expiration_hours: 24,
        stdout: &mut stdout,
    };
    whoami(&mut request).expect("reports");
    assert!(text(&stdout).ends_with("Key expires in: 2.0 hours, renewed on next use\n"));
}

#[rstest]
#[case::older(-89_000.0_f64, 24, true)]
#[case::younger(-85_040.0_f64, 24, false)]
#[case::at_zero_hours(1_000.0_f64, 0, false)]
#[case::above_zero_hours(999.0_f64, 0, true)]
fn whoami_warns_when_the_token_is_older_than_the_expiry_hours(
    #[case] timestamp: f64,
    #[case] expiration_hours: i64,
    #[case] warned: bool,
) {
    let mut stored = record();
    stored.timestamp = timestamp;
    stored.expires_at = None;
    let files = FakeFiles::holding(stored);
    let mut vault = FakeVault::unreachable_read();
    let mut stdout = Vec::new();
    let mut request = WhoamiRequest {
        files: &files,
        vault: &mut vault,
        clock: Arc::new(FixedWallClock(1_000)),
        expiration_hours,
        stdout: &mut stdout,
    };
    whoami(&mut request).expect("reports");
    let warning =
        format!("Warning: Token is more than {expiration_hours} hours old and may have expired.\n");
    assert_eq!(text(&stdout).contains(&warning), warned);
}

fn configure_request<'request>(
    environment: &'request FakeEnvironment,
    files: &'request PrivateFiles,
    api_key: &'request SecretString,
    base_url: &'request str,
    stdout: &'request mut Vec<u8>,
) -> ConfigureClaudeRequest<'request> {
    ConfigureClaudeRequest {
        base_url,
        api_key,
        environment,
        files,
        binary_path: Some(Path::new("/usr/bin/lite")),
        stdout,
    }
}

fn read_settings(path: &Path) -> SettingsDocument {
    serde_json::from_slice(&fs::read(path).expect("reads the settings")).expect("valid JSON")
}

fn env_string<'document>(settings: &'document SettingsDocument, key: &str) -> &'document str {
    settings
        .get("env")
        .and_then(JsonValue::as_object)
        .and_then(|env| env.get(key))
        .and_then(JsonValue::as_str)
        .expect("the env string")
}

fn assert_configured_settings(settings: &Path) {
    let document = read_settings(settings);
    assert_eq!(env_string(&document, "ANTHROPIC_AUTH_TOKEN"), "sk-login");
    assert_eq!(
        env_string(&document, "ANTHROPIC_BASE_URL"),
        "https://gateway.example"
    );
    assert_eq!(env_string(&document, "FOO"), "bar");
    assert!(!document.contains_key("apiKeyHelper"));
    assert_eq!(
        document.get("model").and_then(JsonValue::as_str),
        Some("opus")
    );
    assert_eq!(
        document
            .get("statusLine")
            .and_then(JsonValue::as_object)
            .and_then(|entry| entry.get("command"))
            .and_then(JsonValue::as_str),
        Some("/usr/bin/lite statusline")
    );
    let env = document
        .get("env")
        .and_then(JsonValue::as_object)
        .expect("env object");
    assert!(!env.contains_key("ANTHROPIC_API_KEY"));
}

#[test]
fn configure_claude_writes_a_static_token_and_prints_the_blocks() {
    let home = TempDir::new().expect("home");
    fs::create_dir_all(home.path().join(".claude")).expect("settings directory");
    let settings = home.path().join(".claude").join("settings.json");
    fs::write(
        &settings,
        br#"{"model":"opus","env":{"FOO":"bar"},"apiKeyHelper":"old"}"#,
    )
    .expect("existing settings");
    let environment = FakeEnvironment::at(home.path(), &[]);
    let files = PrivateFiles::default();
    let mut stdout = Vec::new();
    let key = secret("sk-login");
    let mut request = configure_request(
        &environment,
        &files,
        &key,
        "https://gateway.example/",
        &mut stdout,
    );
    configure_claude_code(&mut request).expect("configures");
    assert_eq!(
        text(&stdout),
        format!(
            "\nConfigured Claude Code: {} now routes through https://gateway.example.\n\
             This login's key is stored in the file, so run `lite login --config-claude` again after it expires. Your other Claude Code settings were left untouched. Restart Claude Code to pick this up. Undo with `lite unconfigure claude`; `lite configure claude --model` sets {STARTING_MODEL_ROLE}.\n",
            settings.display()
        )
    );
    assert_configured_settings(&settings);
    assert!(
        home.path()
            .join(".litellm")
            .join("claude_configure_state.json")
            .exists()
    );
}

#[test]
fn configure_claude_follows_the_config_dir() {
    let home = TempDir::new().expect("home");
    let config_dir = TempDir::new().expect("config directory");
    let config = config_dir.path().to_str().expect("a UTF-8 config path");
    let environment = FakeEnvironment::at(home.path(), &[("CLAUDE_CONFIG_DIR", config)]);
    let files = PrivateFiles::default();
    let mut stdout = Vec::new();
    let key = secret("sk-login");
    let mut request = configure_request(
        &environment,
        &files,
        &key,
        "https://gateway.example",
        &mut stdout,
    );
    configure_claude_code(&mut request).expect("configures");
    let settings = config_dir.path().join("settings.json");
    assert_eq!(
        env_string(&read_settings(&settings), "ANTHROPIC_AUTH_TOKEN"),
        "sk-login"
    );
    assert!(!home.path().join(".claude").join("settings.json").exists());
    assert!(text(&stdout).contains(&settings.display().to_string()));
}

#[test]
fn configure_claude_reports_a_settings_failure() {
    let home = TempDir::new().expect("home");
    fs::write(home.path().join(".claude"), b"not a directory").expect("blocking file");
    let environment = FakeEnvironment::at(home.path(), &[]);
    let files = PrivateFiles::default();
    let mut stdout = Vec::new();
    let key = secret("sk-login");
    let mut request = configure_request(
        &environment,
        &files,
        &key,
        "https://gateway.example",
        &mut stdout,
    );
    let error = configure_claude_code(&mut request).expect_err("fails");
    assert!(matches!(error, SessionError::ConfigureClaude { .. }));
    assert!(
        error
            .to_string()
            .starts_with("the login worked, but Claude Code could not be configured: ")
    );
    assert!(stdout.is_empty());
}

#[test]
fn configure_claude_requires_a_home_for_the_state_paths() {
    let config_dir = TempDir::new().expect("config directory");
    let config = config_dir.path().to_str().expect("a UTF-8 config path");
    let environment = FakeEnvironment {
        home: None,
        variables: BTreeMap::from([("CLAUDE_CONFIG_DIR".to_owned(), config.to_owned())]),
    };
    let files = PrivateFiles::default();
    let mut stdout = Vec::new();
    let key = secret("sk-login");
    let mut request = configure_request(
        &environment,
        &files,
        &key,
        "https://gateway.example",
        &mut stdout,
    );
    let error = configure_claude_code(&mut request).expect_err("fails");
    assert!(matches!(error, SessionError::NoHome));
}

#[test]
fn configure_claude_requires_a_home() {
    let environment = FakeEnvironment::homeless();
    let files = PrivateFiles::default();
    let mut stdout = Vec::new();
    let key = secret("sk-login");
    let mut request = configure_request(
        &environment,
        &files,
        &key,
        "https://gateway.example",
        &mut stdout,
    );
    let error = configure_claude_code(&mut request).expect_err("fails");
    assert!(matches!(error, SessionError::NoHome));
}
