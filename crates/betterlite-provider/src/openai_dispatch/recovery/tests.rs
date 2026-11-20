#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::controls::control_options::ControlOptions;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_openai::chat::ChatRequest;
use betterlite_openai::openai_base_config::CatalogCapabilities;
use rstest::rstest;
use secrecy::SecretString;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;
use tokio::time::timeout;
use url::Url;

use crate::openai_dispatch::OpenAi;
use crate::operation::{
    ChatCompletion, ChatOperation, OperationError, OperationRequest, OperationResult,
};
use crate::prepared::PreparedRequest;
use crate::prepared::deployment::Deployment;
use crate::provider::Adapter as _;

const CHAT: &str = r#"{"model":"g","messages":[{"role":"user","content":"hi"}]}"#;
const CHAT_WITH_TEMPERATURE: &str =
    r#"{"model":"g","messages":[{"role":"user","content":"hi"}],"temperature":0.5}"#;
const CHAT_REPLY: &str = r#"{"id":"chatcmpl-1","object":"chat.completion","created":1700000000,"model":"gpt-4o-2024-08-06","choices":[{"index":0,"message":{"role":"assistant","content":"hi"},"logprobs":null,"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}"#;
// LiteLLM's _OUTPUT_TOKEN_LIMIT_ERROR_MARKER inside the provider's own sentence
const LIMIT_ERROR: &str = r#"{"error":{"message":"You could not finish the message because max_tokens or model output limit was reached. Please try again with higher max_tokens.","type":"invalid_request_error","param":"max_tokens","code":"invalid_request_error"}}"#;
const REJECTED_TEMPERATURE: &str = r#"{"error":{"message":"{\"detail\":[{\"loc\":[\"body\",\"temperature\"],\"msg\":\"extra inputs are not permitted\"}]}","type":"invalid_request_error","param":"temperature","code":"invalid_request_error"}}"#;
const MALFORMED_DETAIL: &str = r#"{"error":{"message":"[1, 2]","type":"invalid_request_error","code":"invalid_request_error"}}"#;

type Check = fn() -> Pin<Box<dyn Future<Output = ()>>>;

#[rstest]
#[case::output_limit_chat(output_limit_chat)]
#[case::output_limit_stream(output_limit_stream)]
#[case::unrelated_400_stays_provider(unrelated_400_stays_provider)]
#[case::retry_drops_rejected_fields(retry_drops_rejected_fields)]
#[case::retry_needs_global_flag(retry_needs_global_flag)]
#[case::malformed_detail_fails_the_call(malformed_detail_fails_the_call)]
#[tokio::test]
async fn openai_recovery_cases(#[case] check: Check) {
    check().await;
}

async fn serve(listener: TcpListener, replies: Vec<(u16, &'static str)>) -> Vec<String> {
    let mut bodies = Vec::new();
    for (status, reply) in replies {
        let Ok(accepted) = timeout(Duration::from_secs(2), listener.accept()).await else {
            break;
        };
        let (mut socket, _) = accepted.unwrap();
        let mut head = Vec::new();
        while !head.ends_with(b"\r\n\r\n") {
            head.push(socket.read_u8().await.unwrap());
        }
        let length: usize = String::from_utf8(head)
            .unwrap()
            .lines()
            .find_map(|line| line.strip_prefix("content-length: "))
            .unwrap()
            .parse()
            .unwrap();
        let mut body = vec![0_u8; length];
        socket.read_exact(&mut body).await.unwrap();
        bodies.push(String::from_utf8(body).unwrap());
        let response = format!(
            "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{reply}",
            reply.len()
        );
        socket.write_all(response.as_bytes()).await.unwrap();
    }
    bodies
}

fn client() -> ProxiedClient {
    let policy = ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap();
    ProxiedClient::outbound(&Arc::new(policy)).unwrap()
}

fn deployment(base: &str, global_drop_params: bool) -> Deployment {
    Deployment {
        model: String::from("up"),
        api_base: base.to_owned(),
        endpoint: Url::parse(&format!("{base}/chat/completions")).unwrap(),
        api_key: Some(SecretString::from("configured-key")),
        authorization: None,
        organization: None,
        timeout: Duration::from_secs(5),
        max_retries: None,
        drop_params: None,
        global_drop_params,
        provider_affinity_header: None,
        anthropic: None,
    }
}

fn catalog() -> CatalogCapabilities {
    CatalogCapabilities {
        chat_models: BTreeSet::new(),
        text_models: BTreeSet::new(),
    }
}

fn prepared(request: &str, stream: bool, inputs: &Deployment) -> PreparedRequest {
    let operation = ChatOperation {
        model: String::from("up"),
        request: ChatRequest::parse(request.as_bytes()).unwrap(),
        control: ControlOptions::default(),
    };
    let request = if stream {
        OperationRequest::Stream(Box::new(operation))
    } else {
        OperationRequest::Chat(Box::new(operation))
    };
    OpenAi.prepare(&request, inputs, &catalog()).unwrap()
}

// one prepare, execute and decode against a loopback upstream that answers the given replies in order
async fn answered(
    request: &str,
    stream: bool,
    global_drop_params: bool,
    replies: Vec<(u16, &'static str)>,
) -> (Result<OperationResult, OperationError>, Vec<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    let inputs = deployment(&base, global_drop_params);
    let prepared = prepared(request, stream, &inputs);
    let client = client();
    let (bodies, upstream) = tokio::join!(
        serve(listener, replies),
        OpenAi.execute(&client, &prepared, &inputs)
    );
    let decoded = upstream.and_then(|answer| OpenAi.decode(&prepared, answer));
    (decoded, bodies)
}

fn output_limit_chat() -> Pin<Box<dyn Future<Output = ()>>> {
    Box::pin(async {
        let (result, bodies) = answered(CHAT, false, false, vec![(400, LIMIT_ERROR)]).await;
        let OperationResult::OutputLimit { sent } = result.unwrap() else {
            panic!("the output-limit 400 decodes to the recovered request");
        };
        let sent_on_the_wire: JsonValue = serde_json::from_slice(bodies[0].as_bytes()).unwrap();
        assert_eq!(*sent, sent_on_the_wire);
    })
}

fn output_limit_stream() -> Pin<Box<dyn Future<Output = ()>>> {
    Box::pin(async {
        let request = r#"{"model":"g","messages":[{"role":"user","content":"hi"}],"stream":true}"#;
        let (result, bodies) = answered(request, true, false, vec![(400, LIMIT_ERROR)]).await;
        let OperationResult::OutputLimit { sent } = result.unwrap() else {
            panic!("the output-limit 400 decodes to the recovered request");
        };
        let sent_on_the_wire: JsonValue = serde_json::from_slice(bodies[0].as_bytes()).unwrap();
        assert_eq!(*sent, sent_on_the_wire);
    })
}

fn unrelated_400_stays_provider() -> Pin<Box<dyn Future<Output = ()>>> {
    Box::pin(async {
        let (result, _bodies) = answered(
            CHAT,
            false,
            false,
            vec![(400, r#"{"error":{"message":"bad"}}"#)],
        )
        .await;
        assert!(matches!(result, Err(OperationError::Provider { .. })));
    })
}

fn retry_drops_rejected_fields() -> Pin<Box<dyn Future<Output = ()>>> {
    Box::pin(async {
        let (result, bodies) = answered(
            CHAT_WITH_TEMPERATURE,
            false,
            true,
            vec![(422, REJECTED_TEMPERATURE), (200, CHAT_REPLY)],
        )
        .await;
        let OperationResult::Chat(ChatCompletion::OpenAi(completion)) = result.unwrap() else {
            panic!("the resent call answers the provider's completion");
        };
        assert_eq!(completion.model, "g");
        assert_eq!(bodies.len(), 2);
        assert!(bodies[0].contains("\"temperature\":0.5"), "{}", bodies[0]);
        assert!(!bodies[1].contains("temperature"), "{}", bodies[1]);
        assert!(bodies[1].contains("\"messages\""), "{}", bodies[1]);
        assert!(bodies[1].contains("\"model\":\"up\""), "{}", bodies[1]);
    })
}

fn retry_needs_global_flag() -> Pin<Box<dyn Future<Output = ()>>> {
    Box::pin(async {
        let (result, bodies) = answered(
            CHAT_WITH_TEMPERATURE,
            false,
            false,
            vec![(422, REJECTED_TEMPERATURE), (200, CHAT_REPLY)],
        )
        .await;
        assert!(matches!(result, Err(OperationError::Provider { .. })));
        assert_eq!(bodies.len(), 1);
        assert!(bodies[0].contains("temperature"));
    })
}

fn malformed_detail_fails_the_call() -> Pin<Box<dyn Future<Output = ()>>> {
    Box::pin(async {
        let (result, bodies) = answered(
            CHAT,
            false,
            true,
            vec![(422, MALFORMED_DETAIL), (200, CHAT_REPLY)],
        )
        .await;
        assert!(matches!(result, Err(OperationError::RejectedFields { .. })));
        assert_eq!(bodies.len(), 1);
    })
}
