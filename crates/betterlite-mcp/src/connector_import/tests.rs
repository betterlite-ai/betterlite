#![cfg(test)]

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

use betterlite_config::sections::mcp_env_values::StdioPolicy;
use rstest::rstest;

use super::convert::convert_connector_entries;
use super::request::ConnectorImportRequest;

fn corpus_policy() -> StdioPolicy {
    StdioPolicy {
        enabled: true,
        allowed_commands: ["node".to_owned(), "npx".to_owned()]
            .into_iter()
            .collect::<BTreeSet<String>>(),
    }
}

#[rstest]
fn replays_the_connector_import_request_corpus(
    #[files("fuzz/corpus/connector_import_request/*")] path: PathBuf,
) {
    let bytes = fs::read(path).unwrap();
    let Ok(request) = ConnectorImportRequest::from_slice(&bytes) else {
        return;
    };
    drop(convert_connector_entries(&request, &corpus_policy()));
}
