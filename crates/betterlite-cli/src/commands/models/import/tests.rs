#![cfg(test)]

use std::io::Write as _;

use fancy_regex::Regex;
use indexmap::IndexMap;
use rstest::rstest;
use tempfile::{NamedTempFile, tempdir};

use crate::commands::models::tests::run;
use crate::commands::models::value::ModelObject;
use crate::commands::models::{ImportModels, ModelsCommand, ModelsError};
use crate::commands::tests::support::{Answer, TestServer};

use super::{filter_model, summary_table};

fn yaml(text: &str) -> NamedTempFile {
    let mut file = NamedTempFile::new().unwrap();
    file.write_all(text.as_bytes()).unwrap();
    file
}

fn import(
    file: &NamedTempFile,
    dry_run: bool,
    model_regex: Option<&str>,
    access_regex: Option<&str>,
) -> ModelsCommand {
    ModelsCommand::Import(ImportModels {
        yaml_file: file.path().to_owned(),
        dry_run,
        model_regex: model_regex.map(String::from),
        access_group_regex: access_regex.map(String::from),
    })
}

const TWO_MODELS: &str = "\
model_list:
  - model_name: gpt-4o
    litellm_params:
      model: openai/gpt-4o
    model_info:
      access_groups: [team-a, team-b]
  - model_name: claude
    litellm_params:
      model: anthropic/claude
";

#[test]
fn import_models_prints_the_filtered_models_and_the_provider_summary() {
    let file = yaml(TWO_MODELS);
    let server = TestServer::start(Vec::new());

    let (result, stdout, _stderr) = run(import(&file, true, None, None), &server);

    result.unwrap();
    assert!(
        stdout.contains("Models that would be imported if --dry-run was not provided"),
        "{stdout}"
    );
    assert!(stdout.contains("gpt-4o"), "{stdout}");
    assert!(stdout.contains("openai/gpt-4o"), "{stdout}");
    assert!(stdout.contains("team-a, team-b"), "{stdout}");
    assert!(stdout.contains("claude"), "{stdout}");
    assert!(stdout.contains("Model Import Summary"), "{stdout}");
    assert!(stdout.contains("openai"), "{stdout}");
    assert!(stdout.contains("anthropic"), "{stdout}");
    assert!(stdout.contains("Total"), "{stdout}");
    assert!(server.finish().is_empty());
}

#[test]
fn import_models_posts_each_model_when_not_dry_run() {
    let file = yaml(TWO_MODELS);
    let server = TestServer::start(vec![
        Answer::json(200, r#"{"model_id":"id-1"}"#),
        Answer::json(200, r#"{"model_id":"id-2"}"#),
    ]);

    let (result, stdout, _stderr) = run(import(&file, false, None, None), &server);

    result.unwrap();
    assert!(stdout.contains("Models Imported"), "{stdout}");
    let requests = server.finish();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].request_line, "POST /model/new HTTP/1.1");
    assert_eq!(
        requests[0].body_text(),
        "{\"model_name\":\"gpt-4o\",\"litellm_params\":{\"model\":\"openai/gpt-4o\"},\"model_info\":{\"access_groups\":[\"team-a\",\"team-b\"]}}"
    );
    assert_eq!(
        requests[1].body_text(),
        "{\"model_name\":\"claude\",\"litellm_params\":{\"model\":\"anthropic/claude\"}}"
    );
}

#[test]
fn import_models_ignores_api_errors_and_counts_the_attempt() {
    let file = yaml(TWO_MODELS);
    let server = TestServer::start(vec![
        Answer::json(500, r#"{"detail":"boom"}"#),
        Answer::json(200, "{}"),
    ]);

    let (result, stdout, _stderr) = run(import(&file, false, None, None), &server);

    result.unwrap();
    let requests = server.finish();
    assert_eq!(requests.len(), 2);
    assert!(stdout.contains("Model Import Summary"), "{stdout}");
    let total = stdout
        .lines()
        .find(|line| line.contains("Total"))
        .unwrap_or_default();
    assert!(total.contains('2'), "{stdout}");
}

#[rstest]
#[case::empty("", true)]
#[case::whitespace("  \n", true)]
#[case::other_keys("general:\n  debug: true\n", true)]
#[case::number("model_list: 7\n", false)]
#[case::mapping("model_list: {}\n", false)]
fn import_models_validates_the_model_list(#[case] text: &str, #[case] missing_key: bool) {
    let file = yaml(text);
    let server = TestServer::start(Vec::new());

    let (result, stdout, _stderr) = run(import(&file, true, None, None), &server);

    let matched = match &result {
        Err(ModelsError::MissingModelList) => missing_key,
        Err(ModelsError::ModelListNotList) => !missing_key,
        Ok(()) | Err(_) => false,
    };
    assert!(matched, "{result:?}");
    assert!(stdout.is_empty(), "{stdout}");
    server.finish();
}

#[test]
fn import_models_rejects_malformed_yaml() {
    let file = yaml("model_list: [\n");
    let server = TestServer::start(Vec::new());

    let (result, _stdout, _stderr) = run(import(&file, true, None, None), &server);

    assert!(
        matches!(result, Err(ModelsError::Yaml { .. })),
        "{result:?}"
    );
    server.finish();
}

#[test]
fn import_models_reports_a_missing_file() {
    let directory = tempdir().unwrap();
    let command = ModelsCommand::Import(ImportModels {
        yaml_file: directory.path().join("absent.yaml"),
        dry_run: true,
        model_regex: None,
        access_group_regex: None,
    });
    let server = TestServer::start(Vec::new());

    let (result, _stdout, _stderr) = run(command, &server);

    assert!(
        matches!(result, Err(ModelsError::Read { .. })),
        "{result:?}"
    );
    server.finish();
}

#[test]
fn import_models_filters_by_the_model_regex() {
    let file = yaml(TWO_MODELS);
    let server = TestServer::start(vec![Answer::json(200, "{}")]);

    let (result, stdout, _stderr) = run(import(&file, false, Some("^anthropic/"), None), &server);

    result.unwrap();
    assert!(stdout.contains("claude"), "{stdout}");
    assert!(!stdout.contains("gpt-4o"), "{stdout}");
    let requests = server.finish();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].body_text(),
        "{\"model_name\":\"claude\",\"litellm_params\":{\"model\":\"anthropic/claude\"}}"
    );
}

#[test]
fn import_models_filters_by_the_access_group_regex() {
    let file = yaml(TWO_MODELS);
    let server = TestServer::start(Vec::new());

    let (result, stdout, _stderr) = run(import(&file, true, None, Some("^team-b$")), &server);

    result.unwrap();
    assert!(stdout.contains("gpt-4o"), "{stdout}");
    assert!(!stdout.contains("claude"), "{stdout}");
    server.finish();
}

#[test]
fn import_models_filters_with_a_negative_lookahead() {
    let file = yaml(TWO_MODELS);
    let server = TestServer::start(vec![Answer::json(200, "{}")]);

    let (result, stdout, _stderr) = run(import(&file, false, Some("^(?!openai/)"), None), &server);

    result.unwrap();
    assert!(stdout.contains("claude"), "{stdout}");
    assert!(!stdout.contains("gpt-4o"), "{stdout}");
    assert_eq!(server.finish().len(), 1);
}

#[test]
fn import_models_filters_access_groups_with_a_lookahead() {
    let file = yaml(TWO_MODELS);
    let server = TestServer::start(Vec::new());

    let (result, stdout, _stderr) = run(import(&file, true, None, Some("^team-(?!a)")), &server);

    result.unwrap();
    assert!(stdout.contains("gpt-4o"), "{stdout}");
    assert!(!stdout.contains("claude"), "{stdout}");
    server.finish();
}

#[rstest]
#[case::model_regex(Some("("), None, "--only-models-matching-regex")]
#[case::access_regex(None, Some("("), "--only-access-groups-matching-regex")]
fn import_models_rejects_an_invalid_regex(
    #[case] model_regex: Option<&str>,
    #[case] access_regex: Option<&str>,
    #[case] expected_flag: &str,
) {
    let file = yaml(TWO_MODELS);
    let server = TestServer::start(Vec::new());

    let (result, stdout, _stderr) = run(import(&file, true, model_regex, access_regex), &server);

    assert!(
        matches!(&result, Err(ModelsError::Regex { flag, .. }) if *flag == expected_flag),
        "{result:?}"
    );
    assert!(stdout.is_empty(), "{stdout}");
    server.finish();
}

#[test]
fn import_models_reads_null_access_groups_as_empty() {
    let file = yaml(
        "model_list:\n  - model_name: gpt-4o\n    litellm_params:\n      model: openai/gpt-4o\n    model_info:\n      access_groups: null\n",
    );
    let server = TestServer::start(Vec::new());

    let (result, stdout, _stderr) = run(import(&file, true, None, None), &server);

    result.unwrap();
    let row = stdout
        .lines()
        .find(|line| line.contains("openai/gpt-4o"))
        .unwrap_or_default();
    assert!(
        row.trim_end_matches(|symbol: char| !symbol.is_alphanumeric())
            .ends_with("openai/gpt-4o"),
        "{stdout}"
    );
    server.finish();
}

#[rstest]
#[case::bad_params(
    "      model: openai/gpt-4o\n  - model_name: bad\n    litellm_params: text\n",
    "litellm_params"
)]
#[case::bad_access_groups(
    "      model: openai/gpt-4o\n  - model_name: bad\n    litellm_params:\n      model: openai/x\n    model_info:\n      access_groups: text\n",
    "access_groups"
)]
fn import_models_checks_every_entry_before_posting_any(
    #[case] second_entry: &str,
    #[case] expected_field: &str,
) {
    let file = yaml(&format!(
        "model_list:\n  - model_name: gpt-4o\n    litellm_params:\n{second_entry}"
    ));
    let server = TestServer::start(vec![Answer::json(200, "{}")]);

    let (result, stdout, _stderr) = run(import(&file, false, None, None), &server);

    assert!(
        matches!(&result, Err(ModelsError::EntryField { index: 1, field }) if *field == expected_field),
        "{result:?}"
    );
    assert!(stdout.is_empty(), "{stdout}");
    assert!(server.finish().is_empty());
}

#[test]
fn import_models_takes_the_provider_before_the_first_slash() {
    let file =
        yaml("model_list:\n  - model_name: m\n    litellm_params:\n      model: vertex_ai/x/y\n");
    let server = TestServer::start(Vec::new());

    let (result, stdout, _stderr) = run(import(&file, true, None, None), &server);

    result.unwrap();
    let summary = stdout
        .lines()
        .find(|line| line.contains("vertex_ai") && line.contains('1'))
        .unwrap_or_default();
    assert!(!summary.contains('/'), "{stdout}");
    server.finish();
}

#[test]
fn filter_model_accepts_a_model_with_the_required_fields() {
    let model = object(r#"{"model_name":"m","litellm_params":{"model":"openai/gpt"}}"#);

    assert!(filter_model(&model, None, None, 0).unwrap());
}

#[rstest]
#[case::missing(r#"{"litellm_params":{"model":"openai/gpt"}}"#)]
#[case::null_value(r#"{"model_name":null,"litellm_params":{"model":"openai/gpt"}}"#)]
fn filter_model_rejects_a_model_without_a_model_name(#[case] text: &str) {
    let model = object(text);

    assert!(!filter_model(&model, None, None, 0).unwrap());
}

#[test]
fn filter_model_accepts_a_model_name_that_is_present_but_empty() {
    let model = object(r#"{"model_name":"","litellm_params":{"model":"openai/gpt"}}"#);

    assert!(filter_model(&model, None, None, 0).unwrap());
}

#[test]
fn filter_model_accepts_a_model_id_that_is_present_but_empty() {
    let model = object(r#"{"model_name":"m","litellm_params":{"model":""}}"#);

    assert!(filter_model(&model, None, None, 0).unwrap());
}

#[rstest]
#[case::missing(r#"{"model_name":"m"}"#)]
#[case::null_value(r#"{"model_name":"m","litellm_params":null}"#)]
#[case::non_string_model(r#"{"model_name":"m","litellm_params":{"model":5}}"#)]
fn filter_model_rejects_a_model_without_a_string_model_id(#[case] text: &str) {
    let model = object(text);

    assert!(!filter_model(&model, None, None, 0).unwrap());
}

#[rstest]
#[case::match_model("openai/", true)]
#[case::miss_model("anthropic/", false)]
#[case::negative_lookahead_keeps_other_providers("^(?!azure/)", true)]
#[case::negative_lookahead_drops_the_provider("^(?!openai/)", false)]
fn filter_model_applies_the_model_regex(#[case] pattern: &str, #[case] expected: bool) {
    let model = object(r#"{"model_name":"m","litellm_params":{"model":"openai/gpt"}}"#);
    let regex = Regex::new(pattern).unwrap();

    assert_eq!(
        filter_model(&model, Some(&regex), None, 0).unwrap(),
        expected
    );
}

#[rstest]
#[case::match_group("^b$", r#"{"model_name":"m","litellm_params":{"model":"openai/gpt"},"model_info":{"access_groups":["a","b"]}}"#, true)]
#[case::miss_group("^c$", r#"{"model_name":"m","litellm_params":{"model":"openai/gpt"},"model_info":{"access_groups":["a","b"]}}"#, false)]
#[case::lookahead_hit("^(?!a$)", r#"{"model_name":"m","litellm_params":{"model":"openai/gpt"},"model_info":{"access_groups":["a","b"]}}"#, true)]
#[case::lookahead_miss("^(?!a|b)", r#"{"model_name":"m","litellm_params":{"model":"openai/gpt"},"model_info":{"access_groups":["a","b"]}}"#, false)]
#[case::null_groups(
    "^b$",
    r#"{"model_name":"m","litellm_params":{"model":"openai/gpt"},"model_info":{"access_groups":null}}"#,
    false
)]
#[case::absent_groups(
    "^b$",
    r#"{"model_name":"m","litellm_params":{"model":"openai/gpt"}}"#,
    false
)]
fn filter_model_applies_the_access_group_regex(
    #[case] pattern: &str,
    #[case] text: &str,
    #[case] expected: bool,
) {
    let model = object(text);
    let regex = Regex::new(pattern).unwrap();

    assert_eq!(
        filter_model(&model, None, Some(&regex), 0).unwrap(),
        expected
    );
}

#[test]
fn filter_model_rejects_access_groups_that_are_not_a_list() {
    let model = object(
        r#"{"model_name":"m","litellm_params":{"model":"openai/gpt"},"model_info":{"access_groups":"team"}}"#,
    );
    let regex = Regex::new("team").unwrap();

    assert!(!filter_model(&model, None, Some(&regex), 0).unwrap());
}

#[test]
fn print_models_table_skips_an_empty_model_list() {
    let file = yaml("model_list: []\n");
    let server = TestServer::start(Vec::new());

    let (result, stdout, _stderr) = run(import(&file, true, None, None), &server);

    result.unwrap();
    assert!(!stdout.contains("Model Name"), "{stdout}");
    assert!(!stdout.contains("Upstream Model"), "{stdout}");
    assert!(stdout.contains("Model Import Summary"), "{stdout}");
    server.finish();
}

#[test]
fn print_models_table_lists_names_upstream_models_and_access_groups() {
    let file = yaml(TWO_MODELS);
    let server = TestServer::start(Vec::new());

    let (result, stdout, _stderr) = run(import(&file, true, None, None), &server);

    result.unwrap();
    assert!(stdout.contains("Model Name"), "{stdout}");
    assert!(stdout.contains("Upstream Model"), "{stdout}");
    assert!(stdout.contains("Access Groups"), "{stdout}");
    assert!(stdout.contains("team-a, team-b"), "{stdout}");
    server.finish();
}

#[test]
fn summary_counts_each_provider_and_totals_them() {
    let file = yaml(TWO_MODELS);
    let server = TestServer::start(Vec::new());

    let (result, stdout, _stderr) = run(import(&file, true, None, None), &server);

    result.unwrap();
    let openai = stdout
        .lines()
        .find(|line| line.contains("openai") && !line.contains('/'))
        .unwrap_or_default();
    assert!(openai.contains('1'), "{stdout}");
    let anthropic = stdout
        .lines()
        .find(|line| line.contains("anthropic") && !line.contains('/'))
        .unwrap_or_default();
    assert!(anthropic.contains('1'), "{stdout}");
    let total = stdout
        .lines()
        .find(|line| line.contains("Total"))
        .unwrap_or_default();
    assert!(total.contains('2'), "{stdout}");
    assert!(!total.contains("[bold]"), "{stdout}");
    server.finish();
}

#[test]
fn summary_renders_only_the_total_row_in_bold() {
    let counts = IndexMap::from([
        (String::from("openai"), 1_usize),
        (String::from("azure"), 2_usize),
    ]);
    let mut table = summary_table(&counts).unwrap();
    table.enforce_styling();

    let text = table.to_string();

    let total = text
        .lines()
        .find(|line| line.contains("Total"))
        .unwrap_or_default();
    assert_eq!(total.matches("\u{1b}[1m").count(), 2, "{text:?}");
    for provider in ["openai", "azure"] {
        let row = text
            .lines()
            .find(|line| line.contains(provider))
            .unwrap_or_default();
        assert!(!row.contains('\u{1b}'), "{text:?}");
    }
    assert!(total.contains('3'), "{text:?}");
}

fn object(text: &str) -> ModelObject {
    serde_json::from_str(text).unwrap()
}
