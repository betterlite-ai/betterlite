use betterlite_config::sections::settings::JsonValue;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use clap::Parser as _;
use secrecy::SecretString;

use crate::cli_context::CliContext;
use crate::commands::Client;
use crate::commands::tables::OutputFormat;
use crate::commands::tests::support::{RecordedRequest, TestServer, client};

use super::super::{
    DeleteKeys, GenerateKey, ImportKeys, KeysCommand, KeysError, KeysPorts, ListKeys,
    run_keys_command,
};

pub(super) struct Fixture {
    context: CliContext,
    client: Client,
    redactor: Redactor,
}

impl Fixture {
    pub(super) fn new(server: &TestServer, api_key: Option<&str>) -> Self {
        Self::at(&server.base_url, api_key)
    }

    pub(super) fn at(base_url: &str, api_key: Option<&str>) -> Self {
        let context = CliContext {
            base_url: base_url.to_owned(),
            base_url_explicit: true,
            api_key: api_key.map(|key| SecretString::from(key.to_owned())),
            api_key_from_token_file: false,
        };
        Self {
            client: client(base_url, api_key),
            context,
            redactor: Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)
                .expect("the redaction patterns compile"),
        }
    }
}

pub(super) fn run(
    command: KeysCommand,
    fixture: &Fixture,
) -> (Result<(), KeysError>, String, String) {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let result = run_keys_command(
        command,
        KeysPorts {
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

pub(super) fn list(format: OutputFormat) -> ListKeys {
    ListKeys {
        page: None,
        size: None,
        user_id: None,
        team_id: None,
        organization_id: None,
        key_hash: None,
        key_alias: None,
        return_full_object: true,
        include_team_keys: false,
        format,
    }
}

pub(super) fn generate() -> GenerateKey {
    GenerateKey {
        models: None,
        aliases: None,
        spend: None,
        duration: None,
        key_alias: None,
        team_id: None,
        user_id: None,
        budget_id: None,
        config: None,
    }
}

pub(super) fn delete() -> DeleteKeys {
    DeleteKeys {
        keys: None,
        key_aliases: None,
    }
}

pub(super) fn import(source_base_url: &str) -> ImportKeys {
    ImportKeys {
        source_base_url: source_base_url.to_owned(),
        source_api_key: None,
        dry_run: false,
        created_since: None,
    }
}

pub(super) fn api_key(text: &str) -> SecretString {
    SecretString::from(text.to_owned())
}

pub(super) fn query_pairs(request: &RecordedRequest) -> Vec<(String, String)> {
    let url = url::Url::parse(&format!("http://test{}", request.path())).unwrap();
    url.query_pairs()
        .map(|(name, value)| (name.into_owned(), value.into_owned()))
        .collect()
}

pub(super) fn body_json(request: &RecordedRequest) -> JsonValue {
    serde_json::from_slice(&request.body).unwrap()
}

pub(super) fn value(text: &str) -> JsonValue {
    serde_json::from_str(text).unwrap()
}

pub(super) fn pretty(value: &JsonValue) -> String {
    format!("{}\n", serde_json::to_string_pretty(value).unwrap())
}

pub(super) fn table(title: &str, headers: &[&str], rows: &[&[&str]]) -> String {
    let mut table = comfy_table::Table::new();
    table.set_header(headers.to_vec());
    for row in rows {
        table.add_row(row.to_vec());
    }
    format!("{title}\n{table}\n")
}

pub(super) fn keys_page(keys: &str) -> String {
    format!(r#"{{"keys":[{keys}]}}"#)
}

#[derive(clap::Parser)]
#[command(name = "keys")]
struct KeysCli {
    #[command(subcommand)]
    command: KeysCommand,
}

pub(super) fn parse(arguments: &[&str]) -> KeysCommand {
    KeysCli::try_parse_from(arguments.iter().copied())
        .unwrap()
        .command
}
