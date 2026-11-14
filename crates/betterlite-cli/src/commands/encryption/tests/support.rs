use betterlite_config::sections::settings::JsonValue;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use secrecy::SecretString;

use crate::cli_context::CliContext;
use crate::commands::Client;
use crate::commands::tests::support::{RecordedRequest, TestServer, client};

use super::super::{
    EncryptionCommand, EncryptionError, EncryptionPorts, MigrateEncryption, run_encryption_command,
};

pub(super) struct Fixture {
    context: CliContext,
    client: Client,
    redactor: Redactor,
}

impl Fixture {
    pub(super) fn new(server: &TestServer, api_key: Option<&str>) -> Self {
        let context = CliContext {
            base_url: server.base_url.clone(),
            base_url_explicit: true,
            api_key: api_key.map(|key| SecretString::from(key.to_owned())),
            api_key_from_token_file: false,
        };
        Self {
            client: client(&server.base_url, api_key),
            context,
            redactor: Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)
                .expect("the redaction patterns compile"),
        }
    }
}

pub(super) fn run(
    command: EncryptionCommand,
    fixture: &Fixture,
) -> (Result<(), EncryptionError>, String, String) {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let result = run_encryption_command(
        command,
        EncryptionPorts {
            context: &fixture.context,
            client: &fixture.client,
            redactor: &fixture.redactor,
            stdout: &mut stdout,
            stderr: &mut stderr,
        },
    );
    (
        result,
        String::from_utf8(stdout).unwrap(),
        String::from_utf8(stderr).unwrap(),
    )
}

#[derive(Clone, Copy)]
pub(super) enum Flags {
    None,
    Check,
    DryRun,
    Both,
}

pub(super) fn migrate(flags: Flags) -> MigrateEncryption {
    let (check, dry_run) = match flags {
        Flags::None => (false, false),
        Flags::Check => (true, false),
        Flags::DryRun => (false, true),
        Flags::Both => (true, true),
    };
    MigrateEncryption { check, dry_run }
}

pub(super) fn value(text: &str) -> JsonValue {
    serde_json::from_str(text).unwrap()
}

pub(super) fn pretty(value: &JsonValue) -> String {
    format!("{}\n", serde_json::to_string_pretty(value).unwrap())
}

pub(super) fn query_pairs(request: &RecordedRequest) -> Vec<(String, String)> {
    let url = url::Url::parse(&format!("http://test{}", request.path())).unwrap();
    url.query_pairs()
        .map(|(name, value)| (name.into_owned(), value.into_owned()))
        .collect()
}
