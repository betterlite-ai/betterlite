#![cfg(test)]

use std::io::Write as _;

use regex::Regex;
use rstest::rstest;
use rust_decimal::Decimal;
use tempfile::{NamedTempFile, tempdir};

use crate::commands::tables::OutputFormat;
use crate::commands::tests::support::{Answer, TestServer, client_for};

use super::format::{format_cost_per_1k_tokens, format_iso_datetime_str, format_timestamp};
use super::import::filter_model;
use super::value::ModelObject;
use super::{
    AddModel, DeleteModel, GetModel, GetModelsInfo, ImportModels, ListModels, ModelsCommand,
    ModelsError, UpdateModel, run_models_command,
};

fn run(command: ModelsCommand, server: &TestServer) -> (Result<(), ModelsError>, String, String) {
    let client = client_for(server, Some("sk-test"));
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let result = run_models_command(command, &client, &mut stdout, &mut stderr);
    (
        result,
        String::from_utf8(stdout).unwrap(),
        String::from_utf8(stderr).unwrap(),
    )
}

fn list(format: OutputFormat) -> ModelsCommand {
    ModelsCommand::List(ListModels { format })
}

fn info(columns: &str) -> ModelsCommand {
    ModelsCommand::Info(GetModelsInfo {
        format: OutputFormat::Table,
        columns: columns.to_owned(),
    })
}

fn info_json() -> ModelsCommand {
    ModelsCommand::Info(GetModelsInfo {
        format: OutputFormat::Json,
        columns: String::from("public_model"),
    })
}

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
fn list_models_prints_a_row_for_every_model() {
    let server = TestServer::start(vec![Answer::json(
        200,
        r#"{"data":[{"id":"m1","object":"other","created":1700000000,"owned_by":"openai"},{"id":"m2"}]}"#,
    )]);

    let (result, stdout, _stderr) = run(list(OutputFormat::Table), &server);

    result.unwrap();
    assert!(stdout.contains("Available Models"), "{stdout}");
    assert!(stdout.contains("ID"), "{stdout}");
    assert!(stdout.contains("Object"), "{stdout}");
    assert!(stdout.contains("Created"), "{stdout}");
    assert!(stdout.contains("Owned By"), "{stdout}");
    assert!(stdout.contains("m1"), "{stdout}");
    assert!(stdout.contains("other"), "{stdout}");
    assert!(stdout.contains("openai"), "{stdout}");
    assert!(stdout.contains("2023-11-14 22:13"), "{stdout}");
    assert!(stdout.contains("m2"), "{stdout}");
    assert!(stdout.contains("model"), "{stdout}");
    let requests = server.finish();
    assert_eq!(requests[0].request_line, "GET /models HTTP/1.1");
    assert_eq!(requests[0].header("authorization"), Some("Bearer sk-test"));
    assert_eq!(requests[0].header("content-type"), None);
}

#[test]
fn list_models_json_prints_the_data_array() {
    let server = TestServer::start(vec![Answer::json(200, r#"{"data":[{"id":"m1"}]}"#)]);

    let (result, stdout, _stderr) = run(list(OutputFormat::Json), &server);

    result.unwrap();
    assert_eq!(stdout, "[{\"id\":\"m1\"}]\n");
    server.finish();
}

#[rstest]
#[case::object(r#"{"data":{}}"#)]
#[case::string(r#"{"data":"text"}"#)]
#[case::number(r#"{"data":7}"#)]
#[case::bool(r#"{"data":true}"#)]
#[case::null(r#"{"data":null}"#)]
fn list_models_rejects_data_that_is_not_a_list(#[case] body: &str) {
    let server = TestServer::start(vec![Answer::json(200, body)]);

    let (result, stdout, _stderr) = run(list(OutputFormat::Table), &server);

    assert!(
        matches!(result, Err(ModelsError::ExpectedList { path: "/models" })),
        "{result:?}"
    );
    assert!(stdout.is_empty(), "{stdout}");
    server.finish();
}

#[test]
fn list_models_rejects_a_response_without_data() {
    let server = TestServer::start(vec![Answer::json(200, "{}")]);

    let (result, stdout, _stderr) = run(list(OutputFormat::Table), &server);

    assert!(matches!(result, Err(ModelsError::Response { .. })));
    assert!(stdout.is_empty(), "{stdout}");
    server.finish();
}

#[test]
fn list_models_http_error_is_an_error_without_its_own_handler() {
    let server = TestServer::start(vec![Answer::json(500, r#"{"detail":"boom"}"#)]);

    let (result, stdout, _stderr) = run(list(OutputFormat::Table), &server);

    assert!(matches!(result, Err(ModelsError::Http { status: 500 })));
    assert!(stdout.is_empty(), "{stdout}");
    server.finish();
}

#[test]
fn add_model_splits_each_option_on_the_first_equals() {
    let server = TestServer::start(vec![Answer::json(200, r#"{"model_id":"id-1"}"#)]);
    let command = ModelsCommand::Add(AddModel {
        model_name: String::from("gpt-4o"),
        param: vec![
            String::from("api_base=https://host/v1?x=1"),
            String::from("rpm=10"),
        ],
        info: vec![String::from("team=core")],
    });

    let (result, stdout, _stderr) = run(command, &server);

    result.unwrap();
    assert_eq!(stdout, "{\"model_id\":\"id-1\"}\n");
    let requests = server.finish();
    assert_eq!(requests[0].request_line, "POST /model/new HTTP/1.1");
    assert_eq!(requests[0].header("content-type"), Some("application/json"));
    assert_eq!(
        requests[0].body_text(),
        "{\"model_name\":\"gpt-4o\",\"litellm_params\":{\"api_base\":\"https://host/v1?x=1\",\"rpm\":\"10\"},\"model_info\":{\"team\":\"core\"}}"
    );
}

#[test]
fn add_model_omits_model_info_when_no_info_is_given() {
    let server = TestServer::start(vec![Answer::json(200, "{}")]);
    let command = ModelsCommand::Add(AddModel {
        model_name: String::from("gpt-4o"),
        param: Vec::new(),
        info: Vec::new(),
    });

    let (result, _stdout, _stderr) = run(command, &server);

    result.unwrap();
    let requests = server.finish();
    assert_eq!(
        requests[0].body_text(),
        "{\"model_name\":\"gpt-4o\",\"litellm_params\":{}}"
    );
}

#[test]
fn add_model_rejects_a_param_without_an_equals_sign() {
    let server = TestServer::start(Vec::new());
    let command = ModelsCommand::Add(AddModel {
        model_name: String::from("gpt-4o"),
        param: vec![String::from("broken")],
        info: Vec::new(),
    });

    let (result, stdout, _stderr) = run(command, &server);

    assert!(
        matches!(result, Err(ModelsError::Pair { flag: "--param", value }) if value == "broken"),
        "{result:?}"
    );
    assert!(stdout.is_empty(), "{stdout}");
    assert!(server.finish().is_empty());
}

#[test]
fn add_model_rejects_a_duplicate_param_key() {
    let server = TestServer::start(Vec::new());
    let command = ModelsCommand::Add(AddModel {
        model_name: String::from("gpt-4o"),
        param: vec![String::from("rpm=1"), String::from("rpm=2")],
        info: Vec::new(),
    });

    let (result, _stdout, _stderr) = run(command, &server);

    assert!(
        matches!(result, Err(ModelsError::Duplicate { flag: "--param", key }) if key == "rpm"),
        "{result:?}"
    );
    assert!(server.finish().is_empty());
}

#[test]
fn add_model_http_error_is_an_error_without_its_own_handler() {
    let server = TestServer::start(vec![Answer::json(400, r#"{"detail":"bad"}"#)]);
    let command = ModelsCommand::Add(AddModel {
        model_name: String::from("gpt-4o"),
        param: Vec::new(),
        info: Vec::new(),
    });

    let (result, _stdout, _stderr) = run(command, &server);

    assert!(matches!(result, Err(ModelsError::Http { status: 400 })));
    server.finish();
}

#[test]
fn delete_model_sends_the_id_and_prints_the_response() {
    let server = TestServer::start(vec![Answer::json(200, r#"{"deleted":true}"#)]);
    let command = ModelsCommand::Delete(DeleteModel {
        model_id: String::from("id-1"),
    });

    let (result, stdout, _stderr) = run(command, &server);

    result.unwrap();
    assert_eq!(stdout, "{\"deleted\":true}\n");
    let requests = server.finish();
    assert_eq!(requests[0].request_line, "POST /model/delete HTTP/1.1");
    assert_eq!(requests[0].body_text(), "{\"id\":\"id-1\"}");
}

#[test]
fn delete_model_http_error_is_an_error_without_its_own_handler() {
    let server = TestServer::start(vec![Answer::json(404, r#"{"detail":"missing"}"#)]);
    let command = ModelsCommand::Delete(DeleteModel {
        model_id: String::from("id-1"),
    });

    let (result, _stdout, _stderr) = run(command, &server);

    assert!(matches!(result, Err(ModelsError::Http { status: 404 })));
    server.finish();
}

#[test]
fn update_model_sends_the_id_params_and_info() {
    let server = TestServer::start(vec![Answer::json(200, r#"{"updated":true}"#)]);
    let command = ModelsCommand::Update(UpdateModel {
        model_id: String::from("id-1"),
        param: vec![String::from("model=openai/gpt-4o=latest")],
        info: vec![String::from("team=core")],
    });

    let (result, stdout, _stderr) = run(command, &server);

    result.unwrap();
    assert_eq!(stdout, "{\"updated\":true}\n");
    let requests = server.finish();
    assert_eq!(requests[0].request_line, "POST /model/update HTTP/1.1");
    assert_eq!(
        requests[0].body_text(),
        "{\"id\":\"id-1\",\"litellm_params\":{\"model\":\"openai/gpt-4o=latest\"},\"model_info\":{\"team\":\"core\"}}"
    );
}

#[test]
fn update_model_omits_model_info_when_no_info_is_given() {
    let server = TestServer::start(vec![Answer::json(200, "{}")]);
    let command = ModelsCommand::Update(UpdateModel {
        model_id: String::from("id-1"),
        param: Vec::new(),
        info: Vec::new(),
    });

    let (result, _stdout, _stderr) = run(command, &server);

    result.unwrap();
    let requests = server.finish();
    assert_eq!(
        requests[0].body_text(),
        "{\"id\":\"id-1\",\"litellm_params\":{}}"
    );
}

#[test]
fn get_model_requires_an_id_or_a_name() {
    let server = TestServer::start(Vec::new());
    let command = ModelsCommand::Get(GetModel {
        model_id: None,
        model_name: None,
    });

    let (result, _stdout, _stderr) = run(command, &server);

    assert!(matches!(result, Err(ModelsError::MissingSelector)));
    assert!(server.finish().is_empty());
}

#[test]
fn get_model_rejects_both_an_id_and_a_name() {
    let server = TestServer::start(Vec::new());
    let command = ModelsCommand::Get(GetModel {
        model_id: Some(String::from("id-1")),
        model_name: Some(String::from("gpt-4o")),
    });

    let (result, _stdout, _stderr) = run(command, &server);

    assert!(matches!(result, Err(ModelsError::BothSelectors)));
    assert!(server.finish().is_empty());
}

#[test]
fn get_model_finds_a_model_by_id() {
    let server = TestServer::start(vec![Answer::json(
        200,
        r#"{"data":[{"model_name":"a","model_info":{"id":"id-1"}},{"model_name":"b","model_info":{"id":"id-2"}}]}"#,
    )]);
    let command = ModelsCommand::Get(GetModel {
        model_id: Some(String::from("id-2")),
        model_name: None,
    });

    let (result, stdout, _stderr) = run(command, &server);

    result.unwrap();
    assert_eq!(
        stdout,
        "{\"model_name\":\"b\",\"model_info\":{\"id\":\"id-2\"}}\n"
    );
    let requests = server.finish();
    assert_eq!(requests[0].request_line, "GET /v1/model/info HTTP/1.1");
}

#[test]
fn get_model_finds_a_model_by_name() {
    let server = TestServer::start(vec![Answer::json(
        200,
        r#"{"data":[{"model_name":"a"},{"model_name":"b"}]}"#,
    )]);
    let command = ModelsCommand::Get(GetModel {
        model_id: None,
        model_name: Some(String::from("a")),
    });

    let (result, stdout, _stderr) = run(command, &server);

    result.unwrap();
    assert_eq!(stdout, "{\"model_name\":\"a\"}\n");
    server.finish();
}

#[test]
fn get_model_reports_a_missing_model() {
    let server = TestServer::start(vec![Answer::json(200, r#"{"data":[]}"#)]);
    let command = ModelsCommand::Get(GetModel {
        model_id: Some(String::from("nope")),
        model_name: None,
    });

    let (result, stdout, _stderr) = run(command, &server);

    let Err(error) = result else {
        panic!("expected a not-found error");
    };
    assert!(
        matches!(&error, ModelsError::NotFound { selector } if selector.as_str() == "Model with id=nope"),
        "{error:?}"
    );
    assert_eq!(error.to_string(), "Model with id=nope not found");
    assert!(stdout.is_empty(), "{stdout}");
    server.finish();
}

#[test]
fn info_models_prints_the_default_columns() {
    let server = TestServer::start(vec![Answer::json(
        200,
        r#"{"data":[{"model_name":"gpt-4o","litellm_params":{"model":"openai/gpt-4o"},"model_info":{"updated_at":"2024-01-02T03:04:05Z"}}]}"#,
    )]);

    let (result, stdout, _stderr) = run(info("public_model,upstream_model,updated_at"), &server);

    result.unwrap();
    assert!(stdout.contains("Models Information"), "{stdout}");
    assert!(stdout.contains("Public Model"), "{stdout}");
    assert!(stdout.contains("Upstream Model"), "{stdout}");
    assert!(stdout.contains("Updated At"), "{stdout}");
    assert!(!stdout.contains("Created At"), "{stdout}");
    assert!(stdout.contains("gpt-4o"), "{stdout}");
    assert!(stdout.contains("openai/gpt-4o"), "{stdout}");
    assert!(stdout.contains("2024-01-02 03:04"), "{stdout}");
    let requests = server.finish();
    assert_eq!(requests[0].request_line, "GET /v1/model/info HTTP/1.1");
}

#[test]
fn info_models_renders_the_requested_columns() {
    let server = TestServer::start(vec![Answer::json(
        200,
        r#"{"data":[{"model_name":"gpt-4o","litellm_params":{"model":"openai/gpt-4o","litellm_credential_name":"cred"},"model_info":{"id":"id-1","created_at":"2024-01-02T03:04:05+02:00","input_cost_per_token":0.0000025,"output_cost_per_token":0.5}}]}"#,
    )]);

    let (result, stdout, _stderr) = run(
        info("public_model,credential_name,id,input_cost,output_cost,created_at"),
        &server,
    );

    result.unwrap();
    assert!(stdout.contains("Credential Name"), "{stdout}");
    assert!(stdout.contains("Created At"), "{stdout}");
    assert!(stdout.contains("ID"), "{stdout}");
    assert!(stdout.contains("Input Cost"), "{stdout}");
    assert!(stdout.contains("Output Cost"), "{stdout}");
    assert!(stdout.contains("cred"), "{stdout}");
    assert!(stdout.contains("id-1"), "{stdout}");
    assert!(stdout.contains("$0.0025"), "{stdout}");
    assert!(stdout.contains("$500.0000"), "{stdout}");
    assert!(stdout.contains("2024-01-02 03:04"), "{stdout}");
    server.finish();
}

#[test]
fn info_models_warns_for_unknown_columns_and_renders_the_valid_ones() {
    let server = TestServer::start(vec![Answer::json(
        200,
        r#"{"data":[{"model_name":"gpt-4o"}]}"#,
    )]);

    let (result, stdout, stderr) = run(info(" public_model , nope "), &server);

    result.unwrap();
    assert_eq!(stderr, "Warning: Unknown column 'nope'\n");
    assert!(stdout.contains("Public Model"), "{stdout}");
    assert!(stdout.contains("gpt-4o"), "{stdout}");
    assert!(!stdout.contains("nope"), "{stdout}");
    server.finish();
}

#[test]
fn info_models_json_prints_the_data_array() {
    let server = TestServer::start(vec![Answer::json(
        200,
        r#"{"data":[{"model_name":"gpt-4o"}]}"#,
    )]);

    let (result, stdout, _stderr) = run(info_json(), &server);

    result.unwrap();
    assert_eq!(stdout, "[{\"model_name\":\"gpt-4o\"}]\n");
    server.finish();
}

#[test]
fn cost_formats_a_cost_per_thousand_tokens() {
    assert_eq!(
        format_cost_per_1k_tokens(Some(Decimal::new(25, 7)), "input_cost_per_token").unwrap(),
        "$0.0025"
    );
    assert_eq!(
        format_cost_per_1k_tokens(Some(Decimal::new(3, 1)), "input_cost_per_token").unwrap(),
        "$300.0000"
    );
}

#[test]
fn cost_returns_an_empty_string_for_no_value() {
    assert_eq!(
        format_cost_per_1k_tokens(None, "input_cost_per_token").unwrap(),
        ""
    );
}

#[test]
fn cost_rounds_to_four_decimal_places() {
    assert_eq!(
        format_cost_per_1k_tokens(Some(Decimal::new(1_234_567_89, 11)), "input_cost_per_token")
            .unwrap(),
        "$1.2346"
    );
}

#[test]
fn cost_reports_an_overflowing_quantity() {
    assert!(matches!(
        format_cost_per_1k_tokens(Some(Decimal::MAX), "input_cost_per_token"),
        Err(ModelsError::Cost {
            field: "input_cost_per_token"
        })
    ));
}

#[test]
fn cost_rejects_a_string_cost_naming_the_field() {
    let server = TestServer::start(vec![Answer::json(
        200,
        r#"{"data":[{"model_name":"gpt","model_info":{"input_cost_per_token":"0.5"}}]}"#,
    )]);

    let (result, _stdout, _stderr) = run(info("input_cost"), &server);

    let Err(error) = result else {
        panic!("expected a field-type error");
    };
    assert!(
        matches!(
            error,
            ModelsError::FieldType {
                field: "input_cost_per_token"
            }
        ),
        "{error:?}"
    );
    assert!(error.to_string().contains("input_cost_per_token"));
    server.finish();
}

#[test]
fn cost_reports_a_cost_beyond_the_decimal_range() {
    let server = TestServer::start(vec![Answer::json(
        200,
        r#"{"data":[{"model_name":"gpt","model_info":{"input_cost_per_token":1234567890123456789012345678901234567890}}]}"#,
    )]);

    let (result, _stdout, _stderr) = run(info("input_cost"), &server);

    assert!(
        matches!(
            result,
            Err(ModelsError::Number {
                field: "input_cost_per_token",
                ..
            })
        ),
        "{result:?}"
    );
    server.finish();
}

#[rstest]
#[case::zulu("2024-01-02T03:04:05Z", "2024-01-02 03:04")]
#[case::positive_offset("2024-01-02T03:04:05+02:00", "2024-01-02 03:04")]
#[case::negative_offset("2024-01-02T03:04:59.999-05:30", "2024-01-02 03:04")]
fn iso_formats_rfc3339_datetimes_to_minutes(#[case] text: &str, #[case] expected: &str) {
    assert_eq!(
        format_iso_datetime_str(Some(text), "updated_at").unwrap(),
        expected
    );
}

#[test]
fn iso_returns_an_empty_string_for_no_value() {
    assert_eq!(format_iso_datetime_str(None, "updated_at").unwrap(), "");
}

#[rstest]
#[case::naive("2024-01-02T03:04:05")]
#[case::date_only("2024-01-02")]
#[case::empty("")]
#[case::words("yesterday")]
fn iso_rejects_text_that_is_not_rfc3339(#[case] text: &str) {
    assert!(matches!(
        format_iso_datetime_str(Some(text), "updated_at"),
        Err(ModelsError::DateTime {
            field: "updated_at",
            ..
        })
    ));
}

#[test]
fn iso_rejects_a_non_string_datetime_naming_the_field() {
    let server = TestServer::start(vec![Answer::json(
        200,
        r#"{"data":[{"model_name":"gpt","model_info":{"updated_at":1700000000}}]}"#,
    )]);

    let (result, _stdout, _stderr) = run(info("updated_at"), &server);

    assert!(
        matches!(
            result,
            Err(ModelsError::FieldType {
                field: "updated_at"
            })
        ),
        "{result:?}"
    );
    server.finish();
}

#[test]
fn unix_formats_a_timestamp_to_minutes() {
    assert_eq!(
        format_timestamp(Some(1_700_000_000), "created").unwrap(),
        "2023-11-14 22:13"
    );
}

#[test]
fn unix_returns_an_empty_string_for_no_value() {
    assert_eq!(format_timestamp(None, "created").unwrap(), "");
}

#[test]
fn unix_rejects_a_timestamp_out_of_range() {
    assert!(matches!(
        format_timestamp(Some(i64::MAX), "created"),
        Err(ModelsError::Timestamp { field: "created" })
    ));
}

#[test]
fn unix_reads_the_created_field_as_an_integer() {
    let server = TestServer::start(vec![Answer::json(
        200,
        r#"{"data":[{"id":"m1","created":"1700000000"}]}"#,
    )]);

    let (result, _stdout, _stderr) = run(list(OutputFormat::Table), &server);

    assert!(
        matches!(result, Err(ModelsError::FieldType { field: "created" })),
        "{result:?}"
    );
    server.finish();
}

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
#[case::empty("")]
#[case::whitespace("  \n")]
#[case::other_keys("general:\n  debug: true\n")]
fn import_models_requires_a_model_list_key(#[case] text: &str) {
    let file = yaml(text);
    let server = TestServer::start(Vec::new());

    let (result, stdout, _stderr) = run(import(&file, true, None, None), &server);

    assert!(
        matches!(result, Err(ModelsError::MissingModelList)),
        "{result:?}"
    );
    assert!(stdout.is_empty(), "{stdout}");
    server.finish();
}

#[rstest]
#[case::number("model_list: 7\n")]
#[case::mapping("model_list: {}\n")]
fn import_models_requires_the_model_list_to_be_a_list(#[case] text: &str) {
    let file = yaml(text);
    let server = TestServer::start(Vec::new());

    let (result, _stdout, _stderr) = run(import(&file, true, None, None), &server);

    assert!(
        matches!(result, Err(ModelsError::ModelListNotList)),
        "{result:?}"
    );
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

fn object(text: &str) -> ModelObject {
    serde_json::from_str(text).unwrap()
}
