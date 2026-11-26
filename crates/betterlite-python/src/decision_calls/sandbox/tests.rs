use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_models::decisions::{Extra, Json};
use betterlite_models::sandbox::ContainerHandle;
use betterlite_models::sandbox::HiddenParams as SandboxHiddenParams;
use betterlite_provider::opensandbox::lifecycle::SandboxHandle;
use betterlite_provider::sandbox_dispatch::registry::SandboxTool;
use betterlite_provider::sandbox_dispatch::request::{SandboxContainer, SandboxContainerRef};
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Number;
use serde_json::value::RawValue;

use super::super::convert::{Arguments, ConvertError};
use super::container::{container_answer, container_of};
use super::run::run_request;
use super::{create_request, delete_request, network_policy, tool_answer};

fn e2b_handle() -> ContainerHandle {
    ContainerHandle {
        id: String::from("sbx-1"),
        provider: String::from("e2b"),
        domain: Some(String::from("e2b.app")),
        hidden_params: SandboxHiddenParams {
            envd_access_token: Some(SecretString::from("envd")),
            traffic_access_token: Some(SecretString::from("traffic")),
            api_key: Some(SecretString::from("sk-e2b")),
            api_base: Some(String::from("https://api.e2b.example")),
        },
    }
}

#[test]
fn create_request_carries_every_provider_field() {
    let arguments = Arguments {
        provider: String::from("opensandbox"),
        api_key: Some(SecretString::from("sk-os")),
        api_base: Some(String::from("https://os.example")),
        template: Some(String::from("python")),
        create_timeout: Some(600),
        allow_internet_access: Some(false),
        metadata: Some(BTreeMap::from([(
            String::from("team"),
            String::from("core"),
        )])),
        env_vars: Some(BTreeMap::from([(String::from("A"), String::from("B"))])),
        resource_limits: Some(BTreeMap::from([(String::from("cpu"), String::from("1"))])),
        resource_requests: Some(BTreeMap::from([(
            String::from("memory"),
            String::from("1Gi"),
        )])),
        entrypoint: Some(vec![String::from("python"), String::from("main.py")]),
        secure_access: Some(true),
        use_server_proxy: Some(true),
        ready_timeout: Some(0.5),
        poll_interval: Some(2.0),
        params: Extra::from([(
            String::from("litellm_call_id"),
            Json::Text(String::from("call-1")),
        )]),
        // ast-grep-ignore: no-default-struct-update - the test sets only the fields it reads
        ..Arguments::default()
    };
    let policy = Some(Json::Object(Extra::from([(
        String::from("mode"),
        Json::Text(String::from("allow")),
    )])));
    let raw = network_policy(&policy).unwrap();
    let request = create_request(&arguments, raw.as_deref()).unwrap();
    assert_eq!(request.call.provider, "opensandbox");
    assert_eq!(
        request.call.api_key.map(ExposeSecret::expose_secret),
        Some("sk-os")
    );
    assert_eq!(
        request.call.kwargs.get("litellm_call_id"),
        Some(&Json::Text(String::from("call-1")))
    );
    assert_eq!(request.api_base, Some("https://os.example"));
    assert_eq!(request.template, Some("python"));
    assert_eq!(request.timeout, Some(600));
    assert_eq!(request.allow_internet_access, Some(false));
    assert_eq!(
        request
            .metadata
            .and_then(|values| values.get("team"))
            .map(String::as_str),
        Some("core")
    );
    assert_eq!(
        request
            .env_vars
            .and_then(|values| values.get("A"))
            .map(String::as_str),
        Some("B")
    );
    assert!(request.resource_limits.is_some());
    assert!(request.resource_requests.is_some());
    let entrypoint = request.entrypoint.unwrap();
    assert_eq!(entrypoint.len(), 2);
    assert_eq!(entrypoint[1], "main.py");
    assert_eq!(
        request.network_policy.map(RawValue::get),
        Some(r#"{"mode":"allow"}"#)
    );
    assert!(request.secure_access);
    assert!(request.use_server_proxy);
    assert_eq!(request.ready_timeout, Some(Duration::from_millis(500)));
    assert_eq!(request.poll_interval, Some(Duration::from_secs(2)));
}

#[test]
fn create_request_leaves_the_provider_defaults_alone() {
    let request = create_request(&Arguments::default(), None).unwrap();
    assert_eq!(request.call.provider, "");
    assert!(request.call.api_key.is_none());
    assert!(request.call.kwargs.is_empty());
    assert_eq!(request.api_base, None);
    assert_eq!(request.template, None);
    assert_eq!(request.timeout, None);
    assert_eq!(request.allow_internet_access, None);
    assert_eq!(request.metadata, None);
    assert_eq!(request.network_policy, None);
    assert!(!request.secure_access);
    assert!(!request.use_server_proxy);
    assert_eq!(request.ready_timeout, None);
    assert_eq!(request.poll_interval, None);
}

#[test]
fn run_request_carries_the_code_and_the_container() {
    let container = SandboxContainer::E2b(e2b_handle());
    let arguments = Arguments {
        provider: String::from("e2b"),
        code: String::from("print(1)"),
        language: Some(String::from("python")),
        env_vars: Some(BTreeMap::from([(String::from("A"), String::from("B"))])),
        use_server_proxy: Some(true),
        ready_timeout: Some(1.5),
        api_base: Some(String::from("https://api.e2b.example")),
        // ast-grep-ignore: no-default-struct-update - the test sets only the fields it reads
        ..Arguments::default()
    };
    let request = run_request(&arguments, &container).unwrap();
    assert_eq!(request.code, "print(1)");
    assert_eq!(request.language, Some("python"));
    assert_eq!(
        request
            .env_vars
            .and_then(|values| values.get("A"))
            .map(String::as_str),
        Some("B")
    );
    assert!(request.use_server_proxy);
    assert_eq!(request.ready_timeout, Some(Duration::from_millis(1500)));
    assert_eq!(request.api_base, Some("https://api.e2b.example"));
    assert!(matches!(
        request.container,
        SandboxContainerRef::Container(_)
    ));
}

#[test]
fn delete_request_needs_only_the_container() {
    let container = SandboxContainer::E2b(e2b_handle());
    let arguments = Arguments {
        provider: String::from("e2b"),
        api_base: Some(String::from("https://api.e2b.example")),
        // ast-grep-ignore: no-default-struct-update - the test sets only the fields it reads
        ..Arguments::default()
    };
    let request = delete_request(&arguments, &container);
    assert_eq!(request.call.provider, "e2b");
    assert_eq!(request.api_base, Some("https://api.e2b.example"));
    assert!(matches!(
        request.container,
        SandboxContainerRef::Container(_)
    ));
}

#[test]
fn network_policy_encodes_the_caller_json() {
    assert_eq!(network_policy(&None).unwrap(), None);
    let policy = Some(Json::List(vec![Json::Number(Number::from(1_u32))]));
    assert_eq!(
        network_policy(&policy)
            .unwrap()
            .map(|raw| raw.get().to_owned()),
        Some(String::from("[1]"))
    );
}

#[test]
fn container_answer_writes_the_e2b_tokens_as_hidden_params() {
    let text = container_answer(&SandboxContainer::E2b(e2b_handle())).unwrap();
    let answer: Json = serde_json::from_str(&text).unwrap();
    let Json::Object(fields) = answer else {
        panic!("the answer is a JSON object");
    };
    assert_eq!(fields.get("id"), Some(&Json::Text(String::from("sbx-1"))));
    assert_eq!(
        fields.get("provider"),
        Some(&Json::Text(String::from("e2b")))
    );
    assert_eq!(
        fields.get("domain"),
        Some(&Json::Text(String::from("e2b.app")))
    );
    let Some(Json::Object(hidden)) = fields.get("_hidden_params") else {
        panic!("the answer carries the hidden params");
    };
    assert_eq!(
        hidden.get("envd_access_token"),
        Some(&Json::Text(String::from("envd")))
    );
    assert_eq!(
        hidden.get("traffic_access_token"),
        Some(&Json::Text(String::from("traffic")))
    );
    assert_eq!(
        hidden.get("api_key"),
        Some(&Json::Text(String::from("sk-e2b")))
    );
    assert_eq!(
        hidden.get("api_base"),
        Some(&Json::Text(String::from("https://api.e2b.example")))
    );
}

#[test]
fn container_answer_leaves_absent_e2b_tokens_null() {
    let mut handle = e2b_handle();
    handle.domain = None;
    handle.hidden_params.traffic_access_token = None;
    handle.hidden_params.api_base = None;
    let text = container_answer(&SandboxContainer::E2b(handle)).unwrap();
    let answer: Json = serde_json::from_str(&text).unwrap();
    let Json::Object(fields) = answer else {
        panic!("the answer is a JSON object");
    };
    assert_eq!(fields.get("domain"), Some(&Json::Null));
    let Some(Json::Object(hidden)) = fields.get("_hidden_params") else {
        panic!("the answer carries the hidden params");
    };
    assert_eq!(hidden.get("traffic_access_token"), Some(&Json::Null));
    assert_eq!(hidden.get("api_base"), Some(&Json::Null));
}

#[test]
fn container_answer_opensandbox_keeps_the_execution_endpoint() {
    let handle = SandboxHandle {
        container: ContainerHandle {
            id: String::from("sbx-2"),
            provider: String::from("opensandbox"),
            domain: Some(String::from("https://os.example")),
            hidden_params: SandboxHiddenParams {
                api_key: Some(SecretString::from("sk-os")),
                api_base: None,
                // ast-grep-ignore: no-default-struct-update - the test sets only the fields it reads
                ..SandboxHiddenParams::default()
            },
        },
        execd_endpoint: String::from("https://execd.example"),
        execd_headers: BTreeMap::from([(String::from("x-token"), SecretString::from("abc"))]),
        use_server_proxy: true,
    };
    let text = container_answer(&SandboxContainer::OpenSandbox(handle)).unwrap();
    let answer: Json = serde_json::from_str(&text).unwrap();
    let Json::Object(fields) = answer else {
        panic!("the answer is a JSON object");
    };
    assert_eq!(fields.get("id"), Some(&Json::Text(String::from("sbx-2"))));
    let Some(Json::Object(hidden)) = fields.get("_hidden_params") else {
        panic!("the answer carries the hidden params");
    };
    assert_eq!(
        hidden.get("api_base"),
        Some(&Json::Text(String::from("https://os.example")))
    );
    assert_eq!(
        hidden.get("api_key"),
        Some(&Json::Text(String::from("sk-os")))
    );
    assert_eq!(
        hidden.get("execd_endpoint"),
        Some(&Json::Text(String::from("https://execd.example")))
    );
    let Some(Json::Object(execd_headers)) = hidden.get("execd_headers") else {
        panic!("the execution headers are an object");
    };
    assert_eq!(
        execd_headers.get("x-token"),
        Some(&Json::Text(String::from("abc")))
    );
    assert_eq!(hidden.get("use_server_proxy"), Some(&Json::Bool(true)));
}

#[test]
fn tool_answer_names_the_provider_the_key_and_the_base() {
    assert_eq!(tool_answer(None).unwrap(), "null");
    let tool = SandboxTool {
        provider: String::from("e2b"),
        api_key: Some(SecretString::from("sk-e2b")),
        api_base: None,
    };
    let text = tool_answer(Some(&tool)).unwrap();
    let answer: Json = serde_json::from_str(&text).unwrap();
    let Json::Object(fields) = answer else {
        panic!("the tool is a JSON object");
    };
    assert_eq!(
        fields.get("sandbox_provider"),
        Some(&Json::Text(String::from("e2b")))
    );
    assert_eq!(
        fields.get("api_key"),
        Some(&Json::Text(String::from("sk-e2b")))
    );
    assert_eq!(fields.get("api_base"), Some(&Json::Null));
}

#[test]
fn container_of_reads_a_bare_id_as_an_e2b_handle() {
    let arguments = Arguments {
        provider: String::from("e2b"),
        container: Json::Text(String::from("sbx-3")),
        // ast-grep-ignore: no-default-struct-update - the test sets only the fields it reads
        ..Arguments::default()
    };
    let SandboxContainer::E2b(handle) = container_of(&arguments).unwrap() else {
        panic!("a bare id reads as an e2b handle");
    };
    assert_eq!(handle.id, "sbx-3");
    assert_eq!(handle.provider, "e2b");
    assert_eq!(handle.domain, None);
    assert!(handle.hidden_params.envd_access_token.is_none());
}

#[test]
fn container_of_reads_the_nested_hidden_params() {
    let arguments = Arguments {
        provider: String::from("e2b"),
        use_server_proxy: Some(true),
        container: Json::Object(Extra::from([
            (String::from("id"), Json::Text(String::from("sbx-4"))),
            (
                String::from("hidden_params"),
                Json::Object(Extra::from([
                    (
                        String::from("envd_access_token"),
                        Json::Text(String::from("envd")),
                    ),
                    (String::from("traffic_access_token"), Json::Null),
                    (String::from("api_key"), Json::Text(String::from("sk"))),
                    (
                        String::from("api_base"),
                        Json::Text(String::from("https://e2b.example")),
                    ),
                ])),
            ),
        ])),
        // ast-grep-ignore: no-default-struct-update - the test sets only the fields it reads
        ..Arguments::default()
    };
    let SandboxContainer::E2b(handle) = container_of(&arguments).unwrap() else {
        panic!("an object without an execd endpoint reads as an e2b handle");
    };
    assert_eq!(
        handle
            .hidden_params
            .envd_access_token
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("envd")
    );
    assert!(handle.hidden_params.traffic_access_token.is_none());
    assert_eq!(
        handle
            .hidden_params
            .api_key
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("sk")
    );
    assert_eq!(
        handle.hidden_params.api_base.as_deref(),
        Some("https://e2b.example")
    );
}

#[test]
fn container_of_reads_an_opensandbox_handle_and_its_proxy_flag() {
    let arguments = Arguments {
        provider: String::from("opensandbox"),
        use_server_proxy: Some(true),
        container: Json::Object(Extra::from([
            (String::from("id"), Json::Text(String::from("sbx-5"))),
            (
                String::from("domain"),
                Json::Text(String::from("https://os.example")),
            ),
            (
                String::from("hidden_params"),
                Json::Object(Extra::from([
                    (
                        String::from("execd_endpoint"),
                        Json::Text(String::from("https://execd.example")),
                    ),
                    (
                        String::from("execd_headers"),
                        Json::Object(Extra::from([(
                            String::from("x-token"),
                            Json::Text(String::from("abc")),
                        )])),
                    ),
                    (
                        String::from("api_base"),
                        Json::Text(String::from("https://os.example")),
                    ),
                ])),
            ),
        ])),
        // ast-grep-ignore: no-default-struct-update - the test sets only the fields it reads
        ..Arguments::default()
    };
    let SandboxContainer::OpenSandbox(handle) = container_of(&arguments).unwrap() else {
        panic!("an execd endpoint reads as an opensandbox handle");
    };
    assert_eq!(handle.container.id, "sbx-5");
    assert_eq!(handle.container.provider, "opensandbox");
    assert_eq!(handle.execd_endpoint, "https://execd.example");
    assert!(handle.use_server_proxy);
    assert_eq!(
        handle
            .execd_headers
            .get("x-token")
            .map(ExposeSecret::expose_secret),
        Some("abc")
    );
    assert_eq!(
        handle.container.hidden_params.api_base.as_deref(),
        Some("https://os.example")
    );
}

#[test]
fn container_of_rejects_a_container_with_no_id() {
    let arguments = Arguments {
        container: Json::Bool(true),
        // ast-grep-ignore: no-default-struct-update - the test sets only the fields it reads
        ..Arguments::default()
    };
    assert!(matches!(
        container_of(&arguments),
        Err(ConvertError::Container)
    ));
    let object = Arguments {
        container: Json::Object(Extra::from([(
            String::from("provider"),
            Json::Text(String::from("e2b")),
        )])),
        // ast-grep-ignore: no-default-struct-update - the test sets only the fields it reads
        ..Arguments::default()
    };
    assert!(matches!(
        container_of(&object),
        Err(ConvertError::Container)
    ));
    let numbers = Arguments {
        container: Json::Number(Number::from(1_u32)),
        // ast-grep-ignore: no-default-struct-update - the test sets only the fields it reads
        ..Arguments::default()
    };
    assert!(matches!(
        container_of(&numbers),
        Err(ConvertError::Container)
    ));
}

#[test]
fn container_of_falls_back_to_the_run_proxy_flag() {
    let arguments = Arguments {
        provider: String::from("opensandbox"),
        use_server_proxy: Some(true),
        container: Json::Object(Extra::from([
            (String::from("id"), Json::Text(String::from("sbx-6"))),
            (
                String::from("hidden_params"),
                Json::Object(Extra::from([(
                    String::from("execd_endpoint"),
                    Json::Text(String::from("https://execd.example")),
                )])),
            ),
        ])),
        // ast-grep-ignore: no-default-struct-update - the test sets only the fields it reads
        ..Arguments::default()
    };
    let SandboxContainer::OpenSandbox(handle) = container_of(&arguments).unwrap() else {
        panic!("an execd endpoint reads as an opensandbox handle");
    };
    assert!(handle.use_server_proxy);
    assert!(handle.execd_headers.is_empty());
    assert_eq!(handle.container.provider, "opensandbox");
    assert!(handle.container.hidden_params.api_key.is_none());
}
