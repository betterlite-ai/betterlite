#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::sync::Mutex;

use async_trait::async_trait;
use betterlite_models::sandbox::{ContainerHandle, HiddenParams};
use futures_util::{StreamExt as _, stream};
use http::Method;
use http::header::CONTENT_TYPE;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use super::operations::CreateBody;
use super::operations::RunBody;
use super::parse::parse_ndjson;
use super::{
    CreateSandbox, DeleteSandbox, E2B_API_BASE, E2B_DEFAULT_DOMAIN, E2B_DEFAULT_TEMPLATE, E2bError,
    E2bSandbox, RunCode, SandboxHttp, SandboxRef, SandboxRequest, SandboxResponse, SandboxStream,
    check_status,
};

type Environment = BTreeMap<String, String>;

struct FakeHttp {
    status: u16,
    body: Vec<u8>,
    requests: Mutex<Vec<SandboxRequest>>,
}

impl FakeHttp {
    fn requests(&self) -> Vec<SandboxRequest> {
        self.requests.lock().unwrap().clone()
    }

    fn only_request(&self) -> SandboxRequest {
        let mut requests = self.requests();
        assert_eq!(requests.len(), 1);
        requests.remove(0)
    }
}

#[async_trait]
impl SandboxHttp for FakeHttp {
    async fn send_buffered(&self, request: SandboxRequest) -> Result<SandboxResponse, E2bError> {
        self.requests.lock().unwrap().push(request);
        Ok(SandboxResponse {
            status: self.status,
            body: self.body.clone(),
        })
    }

    async fn send_streaming(&self, request: SandboxRequest) -> Result<SandboxStream, E2bError> {
        self.requests.lock().unwrap().push(request);
        Ok(SandboxStream {
            status: self.status,
            body: stream::iter(vec![Ok::<Vec<u8>, E2bError>(self.body.clone())]).boxed(),
        })
    }
}

fn fake(status: u16, body: &str) -> FakeHttp {
    FakeHttp {
        status,
        body: body.as_bytes().to_vec(),
        requests: Mutex::new(Vec::new()),
    }
}

fn handle(envd: Option<&str>, traffic: Option<&str>) -> ContainerHandle {
    ContainerHandle {
        id: "sbx_1".to_owned(),
        provider: "e2b".to_owned(),
        domain: Some("custom.e2b.app".to_owned()),
        hidden_params: HiddenParams {
            envd_access_token: envd.map(SecretString::from),
            traffic_access_token: traffic.map(SecretString::from),
            api_key: None,
            api_base: None,
        },
    }
}

fn environment(entries: &[(&str, &str)]) -> Environment {
    entries
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect()
}

#[test]
fn validate_environment_requires_e2b_api_key() {
    let sandbox = E2bSandbox {
        http: fake(200, ""),
        environment: Environment::new(),
    };
    let error = sandbox.validate_environment(None).unwrap_err();
    assert!(error.to_string().contains("E2B_API_KEY"));
}

#[test]
fn validate_environment_reads_the_api_key_from_the_environment() {
    let sandbox = E2bSandbox {
        http: fake(200, ""),
        environment: environment(&[("E2B_API_KEY", "env-key")]),
    };
    assert_eq!(
        sandbox.validate_environment(None).unwrap().expose_secret(),
        "env-key"
    );
}

#[test]
fn validate_environment_treats_an_empty_variable_as_unset() {
    let sandbox = E2bSandbox {
        http: fake(200, ""),
        environment: environment(&[("E2B_API_KEY", "")]),
    };
    assert!(matches!(
        sandbox.validate_environment(None).unwrap_err(),
        E2bError::MissingApiKey
    ));
}

#[test]
fn validate_environment_uses_an_explicit_empty_key_as_given() {
    let sandbox = E2bSandbox {
        http: fake(200, ""),
        environment: environment(&[("E2B_API_KEY", "env-key")]),
    };
    assert_eq!(
        sandbox
            .validate_environment(Some(""))
            .unwrap()
            .expose_secret(),
        ""
    );
}

#[rstest]
#[case::success(200, None)]
#[case::success_at_the_top(299, None)]
#[case::redirect_is_not_an_error(302, None)]
#[case::client_error(400, Some(400))]
#[case::not_found(404, Some(404))]
#[case::server_error(500, Some(500))]
fn statuses_at_400_and_up_are_errors(#[case] status: u16, #[case] wanted: Option<u16>) {
    match (check_status(status), wanted) {
        (Ok(()), None) => {}
        (Err(E2bError::HttpStatus { status: found }), Some(expected)) => {
            assert_eq!(found, expected);
        }
        (result, expected) => panic!("unexpected {result:?} for {expected:?}"),
    }
}

#[tokio::test]
async fn create_posts_defaults_and_stores_the_access_tokens() {
    let body = r#"{"sandboxID":"sbx_1","domain":"custom.e2b.app","envdAccessToken":"envd-token","trafficAccessToken":"traffic-token"}"#;
    let sandbox = E2bSandbox {
        http: fake(200, body),
        environment: Environment::new(),
    };
    let created = sandbox
        .acreate_sandbox(CreateSandbox {
            template: None,
            timeout: None,
            allow_internet_access: None,
            api_key: Some("e2b-key"),
            api_base: None,
        })
        .await
        .unwrap();
    assert_created_tokens(&created);
    assert_create_request(&sandbox.http.only_request());
}

fn assert_created_tokens(created: &ContainerHandle) {
    assert_eq!(created.id, "sbx_1");
    assert_eq!(created.provider, "e2b");
    assert_eq!(created.domain.as_deref(), Some("custom.e2b.app"));
    assert_eq!(
        created
            .hidden_params
            .envd_access_token
            .as_ref()
            .unwrap()
            .expose_secret(),
        "envd-token"
    );
    assert_eq!(
        created
            .hidden_params
            .traffic_access_token
            .as_ref()
            .unwrap()
            .expose_secret(),
        "traffic-token"
    );
    assert_eq!(
        created
            .hidden_params
            .api_key
            .as_ref()
            .unwrap()
            .expose_secret(),
        "e2b-key"
    );
    assert_eq!(
        created.hidden_params.api_base.as_deref(),
        Some(E2B_API_BASE)
    );
}

fn assert_create_request(request: &SandboxRequest) {
    assert_eq!(request.method, Method::POST);
    assert_eq!(request.url.as_str(), "https://api.e2b.app/sandboxes");
    let key = request.headers.get("x-api-key").unwrap();
    assert_eq!(key.to_str().unwrap(), "e2b-key");
    assert!(key.is_sensitive());
    assert_eq!(
        request.headers.get(CONTENT_TYPE).unwrap(),
        "application/json"
    );
    let sent: CreateBody = serde_json::from_slice(&request.body).unwrap();
    assert_eq!(sent.template_id, E2B_DEFAULT_TEMPLATE);
    assert_eq!(sent.timeout, 300);
    assert!(sent.secure);
    assert!(sent.allow_internet_access);
}

#[tokio::test]
async fn create_uses_the_template_timeout_internet_choice_and_api_base() {
    let sandbox = E2bSandbox {
        http: fake(200, r#"{"sandboxID":"sbx_2"}"#),
        environment: Environment::new(),
    };
    let created = sandbox
        .acreate_sandbox(CreateSandbox {
            template: Some("my-template"),
            timeout: Some(42),
            allow_internet_access: Some(false),
            api_key: Some("e2b-key"),
            api_base: Some("http://127.0.0.1:9"),
        })
        .await
        .unwrap();
    assert_eq!(created.domain.as_deref(), Some(E2B_DEFAULT_DOMAIN));
    let request = sandbox.http.only_request();
    assert_eq!(request.url.as_str(), "http://127.0.0.1:9/sandboxes");
    let sent: CreateBody = serde_json::from_slice(&request.body).unwrap();
    assert_eq!(sent.template_id, "my-template");
    assert_eq!(sent.timeout, 42);
    assert!(!sent.allow_internet_access);
}

#[tokio::test]
async fn create_reads_the_key_from_the_environment_when_the_caller_passes_none() {
    let sandbox = E2bSandbox {
        http: fake(200, r#"{"sandboxID":"sbx_3"}"#),
        environment: environment(&[("E2B_API_KEY", "env-key")]),
    };
    sandbox
        .acreate_sandbox(CreateSandbox {
            template: None,
            timeout: None,
            allow_internet_access: None,
            api_key: None,
            api_base: None,
        })
        .await
        .unwrap();
    let request = sandbox.http.only_request();
    assert_eq!(
        request.headers.get("x-api-key").unwrap().to_str().unwrap(),
        "env-key"
    );
}

#[tokio::test]
async fn create_stops_before_the_request_without_a_key() {
    let sandbox = E2bSandbox {
        http: fake(200, r#"{"sandboxID":"sbx_3"}"#),
        environment: Environment::new(),
    };
    assert!(matches!(
        sandbox
            .acreate_sandbox(CreateSandbox {
                template: None,
                timeout: None,
                allow_internet_access: None,
                api_key: None,
                api_base: None,
            })
            .await
            .unwrap_err(),
        E2bError::MissingApiKey
    ));
    assert!(sandbox.http.requests().is_empty());
}

#[tokio::test]
async fn create_reports_an_error_status() {
    let sandbox = E2bSandbox {
        http: fake(500, "the API is down"),
        environment: Environment::new(),
    };
    assert!(matches!(
        sandbox
            .acreate_sandbox(CreateSandbox {
                template: None,
                timeout: None,
                allow_internet_access: None,
                api_key: Some("e2b-key"),
                api_base: None,
            })
            .await
            .unwrap_err(),
        E2bError::HttpStatus { status: 500 }
    ));
}

#[tokio::test]
async fn create_reports_a_response_that_is_not_json() {
    let sandbox = E2bSandbox {
        http: fake(200, "not json"),
        environment: Environment::new(),
    };
    assert!(matches!(
        sandbox
            .acreate_sandbox(CreateSandbox {
                template: None,
                timeout: None,
                allow_internet_access: None,
                api_key: Some("e2b-key"),
                api_base: None,
            })
            .await
            .unwrap_err(),
        E2bError::CreateResponse { .. }
    ));
}

#[tokio::test]
async fn run_posts_to_the_jupyter_endpoint_with_both_tokens() {
    let container = handle(Some("envd-token"), Some("traffic-token"));
    let sandbox = E2bSandbox {
        http: fake(200, ""),
        environment: Environment::new(),
    };
    sandbox
        .arun_code(RunCode {
            container: &container,
            code: "print(1)",
            env_vars: None,
        })
        .await
        .unwrap();
    assert_run_request(&sandbox.http.only_request());
}

fn assert_run_request(request: &SandboxRequest) {
    assert_eq!(request.method, Method::POST);
    assert_eq!(
        request.url.as_str(),
        "https://49999-sbx_1.custom.e2b.app/execute"
    );
    assert_eq!(
        request
            .headers
            .get("x-access-token")
            .unwrap()
            .to_str()
            .unwrap(),
        "envd-token"
    );
    assert_eq!(
        request
            .headers
            .get("e2b-traffic-access-token")
            .unwrap()
            .to_str()
            .unwrap(),
        "traffic-token"
    );
    assert_eq!(
        request.headers.get(CONTENT_TYPE).unwrap(),
        "application/json"
    );
    let sent: RunBody = serde_json::from_slice(&request.body).unwrap();
    assert_eq!(sent.code, "print(1)");
    assert_eq!(sent.context_id, None);
    assert!(sent.env_vars.is_none());
}

#[tokio::test]
async fn run_omits_the_traffic_header_without_a_traffic_token() {
    let container = handle(Some("envd-token"), None);
    let sandbox = E2bSandbox {
        http: fake(200, ""),
        environment: Environment::new(),
    };
    sandbox
        .arun_code(RunCode {
            container: &container,
            code: "1",
            env_vars: None,
        })
        .await
        .unwrap();
    let request = sandbox.http.only_request();
    assert!(!request.headers.contains_key("e2b-traffic-access-token"));
}

#[tokio::test]
async fn run_forwards_environment_variables() {
    let container = handle(Some("envd-token"), None);
    let env_vars = environment(&[("FOO", "bar"), ("BAZ", "qux")]);
    let sandbox = E2bSandbox {
        http: fake(200, ""),
        environment: Environment::new(),
    };
    sandbox
        .arun_code(RunCode {
            container: &container,
            code: "1",
            env_vars: Some(&env_vars),
        })
        .await
        .unwrap();
    let request = sandbox.http.only_request();
    let sent: RunBody = serde_json::from_slice(&request.body).unwrap();
    assert_eq!(sent.env_vars.as_ref(), Some(&env_vars));
}

#[tokio::test]
async fn run_without_an_access_token_names_the_container_handle() {
    let container = handle(None, None);
    let sandbox = E2bSandbox {
        http: fake(200, ""),
        environment: Environment::new(),
    };
    let error = sandbox
        .arun_code(RunCode {
            container: &container,
            code: "1",
            env_vars: None,
        })
        .await
        .unwrap_err();
    assert!(matches!(error, E2bError::MissingAccessToken));
    assert!(error.to_string().contains("ContainerHandle"));
    assert!(sandbox.http.requests().is_empty());
}

#[tokio::test]
async fn run_returns_the_first_error_and_execution_count_in_encounter_order() {
    let body = concat!(
        r#"{"type":"stdout","text":"hello "}"#,
        "\n",
        r#"{"type":"stderr","text":"warn\n"}"#,
        "\n",
        r#"{"type":"stdout","text":"world"}"#,
        "\n",
        r#"{"type":"number_of_executions","execution_count":3}"#,
        "\n",
        r#"{"type":"error","name":"ValueError","value":"bad input","traceback":["line"]}"#,
        "\n",
        r#"{"type":"error","name":"Second","value":"ignored"}"#,
        "\n",
        r#"{"type":"number_of_executions","execution_count":9}"#,
    );
    let container = handle(Some("envd-token"), None);
    let sandbox = E2bSandbox {
        http: fake(200, body),
        environment: Environment::new(),
    };
    let result = sandbox
        .arun_code(RunCode {
            container: &container,
            code: "1",
            env_vars: None,
        })
        .await
        .unwrap();
    assert_eq!(result.stdout, "hello world");
    assert_eq!(result.stderr, "warn\n");
    assert_eq!(result.execution_count, Some(3));
    assert_eq!(result.object, "code_execution");
    let error = result.error.unwrap();
    assert_eq!(error.name.unwrap().get(), r#""ValueError""#);
    assert_eq!(error.value.unwrap().get(), r#""bad input""#);
    assert_eq!(error.traceback.unwrap().get(), r#"["line"]"#);
}

#[tokio::test]
async fn run_returns_results_without_the_type_field() {
    let body = concat!(
        r#"{"type":"result","data":{"x":1},"text":"repr"}"#,
        "\n",
        r#"{"type":"result","data":{"y":2}}"#,
    );
    let container = handle(Some("envd-token"), None);
    let sandbox = E2bSandbox {
        http: fake(200, body),
        environment: Environment::new(),
    };
    let result = sandbox
        .arun_code(RunCode {
            container: &container,
            code: "1",
            env_vars: None,
        })
        .await
        .unwrap();
    assert_eq!(result.results.len(), 2);
    assert!(!result.results[0].contains_key("type"));
    assert_eq!(result.results[0]["data"].get(), r#"{"x":1}"#);
    assert_eq!(result.results[0]["text"].get(), r#""repr""#);
    assert_eq!(result.results[1]["data"].get(), r#"{"y":2}"#);
}

#[tokio::test]
async fn run_skips_malformed_and_non_object_lines() {
    let body = concat!(
        "not json",
        "\n",
        "[]",
        "\n",
        "0",
        "\n",
        "null",
        "\n",
        r#"{"type":"unknown","text":"ignored"}"#,
        "\n",
        r#"{"type":"stdout","text":"kept"}"#,
    );
    let container = handle(Some("envd-token"), None);
    let sandbox = E2bSandbox {
        http: fake(200, body),
        environment: Environment::new(),
    };
    let result = sandbox
        .arun_code(RunCode {
            container: &container,
            code: "1",
            env_vars: None,
        })
        .await
        .unwrap();
    assert_eq!(result.stdout, "kept");
    assert!(result.stderr.is_empty());
    assert!(result.results.is_empty());
    assert!(result.error.is_none());
}

#[tokio::test]
async fn run_reports_an_error_status_before_parsing() {
    let container = handle(Some("envd-token"), None);
    let sandbox = E2bSandbox {
        http: fake(404, r#"{"type":"stdout","text":"never parsed"}"#),
        environment: Environment::new(),
    };
    assert!(matches!(
        sandbox
            .arun_code(RunCode {
                container: &container,
                code: "1",
                env_vars: None,
            })
            .await
            .unwrap_err(),
        E2bError::HttpStatus { status: 404 }
    ));
}

#[tokio::test]
async fn run_stops_when_the_output_passes_the_ten_mib_cap() {
    let body = "a".repeat(10 * 1024 * 1024 + 1);
    let container = handle(Some("envd-token"), None);
    let sandbox = E2bSandbox {
        http: fake(200, &body),
        environment: Environment::new(),
    };
    let error = sandbox
        .arun_code(RunCode {
            container: &container,
            code: "1",
            env_vars: None,
        })
        .await
        .unwrap_err();
    assert!(matches!(error, E2bError::OutputTooLarge { limit } if limit == 10 * 1024 * 1024));
    assert!(error.to_string().contains("10485760"));
}

#[tokio::test]
async fn run_reports_output_that_is_not_utf8() {
    let container = handle(Some("envd-token"), None);
    let sandbox = E2bSandbox {
        http: FakeHttp {
            status: 200,
            body: vec![0xff, b'\n'],
            requests: Mutex::new(Vec::new()),
        },
        environment: Environment::new(),
    };
    assert!(matches!(
        sandbox
            .arun_code(RunCode {
                container: &container,
                code: "1",
                env_vars: None,
            })
            .await
            .unwrap_err(),
        E2bError::OutputNotUtf8 { .. }
    ));
}

#[tokio::test]
async fn delete_uses_the_stored_credentials_and_returns_true() {
    let container = ContainerHandle {
        id: "sbx_1".to_owned(),
        provider: "e2b".to_owned(),
        domain: Some("custom.e2b.app".to_owned()),
        hidden_params: HiddenParams {
            envd_access_token: None,
            traffic_access_token: None,
            api_key: Some(SecretString::from("stored-key")),
            api_base: Some("https://stored.example".to_owned()),
        },
    };
    let sandbox = E2bSandbox {
        http: fake(200, ""),
        environment: Environment::new(),
    };
    let deleted = sandbox
        .adelete_sandbox(DeleteSandbox {
            container: SandboxRef::Handle(&container),
            api_key: None,
            api_base: None,
        })
        .await
        .unwrap();
    assert!(deleted);
    let request = sandbox.http.only_request();
    assert_eq!(request.method, Method::DELETE);
    assert_eq!(
        request.url.as_str(),
        "https://stored.example/sandboxes/sbx_1"
    );
    assert_eq!(
        request.headers.get("x-api-key").unwrap().to_str().unwrap(),
        "stored-key"
    );
    assert!(request.body.is_empty());
}

#[tokio::test]
async fn delete_prefers_the_parameters_over_the_stored_credentials() {
    let container = ContainerHandle {
        id: "sbx_1".to_owned(),
        provider: "e2b".to_owned(),
        domain: None,
        hidden_params: HiddenParams {
            envd_access_token: None,
            traffic_access_token: None,
            api_key: Some(SecretString::from("stored-key")),
            api_base: Some("https://stored.example".to_owned()),
        },
    };
    let sandbox = E2bSandbox {
        http: fake(200, ""),
        environment: Environment::new(),
    };
    sandbox
        .adelete_sandbox(DeleteSandbox {
            container: SandboxRef::Handle(&container),
            api_key: Some("param-key"),
            api_base: Some("https://param.example"),
        })
        .await
        .unwrap();
    let request = sandbox.http.only_request();
    assert_eq!(
        request.url.as_str(),
        "https://param.example/sandboxes/sbx_1"
    );
    assert_eq!(
        request.headers.get("x-api-key").unwrap().to_str().unwrap(),
        "param-key"
    );
}

#[tokio::test]
async fn delete_accepts_a_bare_id_with_parameter_credentials() {
    let sandbox = E2bSandbox {
        http: fake(200, ""),
        environment: Environment::new(),
    };
    assert!(
        sandbox
            .adelete_sandbox(DeleteSandbox {
                container: SandboxRef::Id("sbx_9"),
                api_key: Some("param-key"),
                api_base: Some("https://api.e2b.app"),
            })
            .await
            .unwrap()
    );
    let request = sandbox.http.only_request();
    assert_eq!(request.url.as_str(), "https://api.e2b.app/sandboxes/sbx_9");
}

#[tokio::test]
async fn delete_reads_the_key_from_the_environment_for_a_bare_id() {
    let sandbox = E2bSandbox {
        http: fake(200, ""),
        environment: environment(&[("E2B_API_KEY", "env-key")]),
    };
    sandbox
        .adelete_sandbox(DeleteSandbox {
            container: SandboxRef::Id("sbx_9"),
            api_key: None,
            api_base: Some("https://api.e2b.app"),
        })
        .await
        .unwrap();
    let request = sandbox.http.only_request();
    assert_eq!(
        request.headers.get("x-api-key").unwrap().to_str().unwrap(),
        "env-key"
    );
}

#[tokio::test]
async fn delete_returns_false_on_not_found() {
    let container = handle(None, None);
    let sandbox = E2bSandbox {
        http: fake(404, ""),
        environment: environment(&[("E2B_API_KEY", "env-key")]),
    };
    assert!(
        !sandbox
            .adelete_sandbox(DeleteSandbox {
                container: SandboxRef::Handle(&container),
                api_key: None,
                api_base: None,
            })
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn delete_returns_false_for_a_redirect_response() {
    let container = handle(None, None);
    let sandbox = E2bSandbox {
        http: fake(302, ""),
        environment: environment(&[("E2B_API_KEY", "env-key")]),
    };
    assert!(
        !sandbox
            .adelete_sandbox(DeleteSandbox {
                container: SandboxRef::Handle(&container),
                api_key: None,
                api_base: None,
            })
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn delete_reports_other_error_statuses() {
    let container = handle(None, None);
    let sandbox = E2bSandbox {
        http: fake(500, ""),
        environment: environment(&[("E2B_API_KEY", "env-key")]),
    };
    assert!(matches!(
        sandbox
            .adelete_sandbox(DeleteSandbox {
                container: SandboxRef::Handle(&container),
                api_key: None,
                api_base: None,
            })
            .await
            .unwrap_err(),
        E2bError::HttpStatus { status: 500 }
    ));
}

#[test]
fn parse_ndjson_reads_every_output_kind_in_one_pass() {
    let lines: Vec<String> = vec![
        r#"{"type":"stdout","text":"a"}"#.to_owned(),
        r#"{"type":"stderr","text":"b"}"#.to_owned(),
        r#"{"type":"stdout","text":"c"}"#.to_owned(),
    ];
    assert_eq!(parse_ndjson(&lines).stdout, "ac");
}

#[test]
fn replays_the_sandbox_ndjson_corpus() {
    let seen = fs::read_dir("fuzz/corpus/sandbox_ndjson")
        .unwrap()
        .map(|entry| fs::read_to_string(entry.unwrap().path()).unwrap())
        .map(|body| {
            let lines: Vec<String> = body.lines().map(str::to_owned).collect();
            parse_ndjson(&lines)
        })
        .count();
    assert!(seen > 0_usize);
}
