#![cfg(test)]

use std::io;
use std::path::PathBuf;
use std::{fs, str};

use rstest::rstest;

use super::GitLabTemplateManager;
use super::error::ManagerError;
use super::manager_init::ClientSource;
use super::parse::{FrontmatterParser, parse_prompt_file, parse_yaml_basic};
use super::sandbox::render_sandboxed;
use crate::arguments::json::Object;
use crate::gitlab_client::payload::{decode_file_payload, decode_text};
use crate::gitlab_client::tests::{FakeTransport, config_from};
use crate::gitlab_client::transport::GitLabResponse;

pub(super) const SCOPED_CONFIG: &str =
    r#"{"project": "g/r", "access_token": "t", "prompts_path": "prompts"}"#;

pub(super) type Scripted = Vec<Result<GitLabResponse, io::Error>>;

// Builds a manager over a scripted transport and hands the transport back to inspect the requests.
pub(super) async fn build_manager(
    config_json: &str,
    prompt_id: Option<&str>,
    ref_name: Option<&str>,
    script: Scripted,
) -> (
    Result<GitLabTemplateManager<FakeTransport>, ManagerError>,
    FakeTransport,
) {
    let transport = FakeTransport::scripted(script);
    let manager = GitLabTemplateManager::new(
        &config_from(config_json),
        prompt_id,
        ref_name,
        ClientSource::Transport(transport.clone()),
        FrontmatterParser::Yaml,
    )
    .await;
    (manager, transport)
}

#[rstest]
fn replays_the_gitlab_templates_corpus(#[files("fuzz/corpus/gitlab_templates/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    if bytes.len() > 0x0001_0000 {
        return;
    }
    drop(decode_file_payload(&bytes));
    drop(decode_text(&bytes));
    let Ok(content) = str::from_utf8(&bytes) else {
        return;
    };
    drop(parse_yaml_basic(content));
    for parser in [FrontmatterParser::Yaml, FrontmatterParser::Basic] {
        if let Ok(template) = parse_prompt_file(content, "fuzz", parser) {
            drop(render_sandboxed("fuzz", &template.content, &Object::new()));
        }
    }
}
