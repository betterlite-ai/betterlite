use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use betterlite_openai::huggingface_chat::error::HuggingfaceChatError;
use betterlite_openai::huggingface_chat::mapping::ProviderMappingSource;
use betterlite_prompts::factory::custom::CustomPromptTemplate;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;
use tokio::time::timeout;

use super::CustomPromptTable;
use super::embedding::NoTag;
use super::hub::PooledHub;
use super::huggingface::NoMapping;
use super::transport::PooledPost;
use crate::api::rerank_contracts::{
    MapCohereRerankParams, Params as RerankParams, RerankLogging, RerankProviderResponse,
    ResponseTransform,
};
use betterlite_openai::api::rerank_contracts::records::RerankResponse;

const EXCHANGE_TIMEOUT: Duration = Duration::from_secs(5);
const TEMPLATE: &str = r#"{"roles":{"user":{"pre_message":"U:","post_message":"!"}},"initial_prompt_value":"I","final_prompt_value":"F"}"#;

fn client() -> ProxiedClient {
    let policy = ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap();
    ProxiedClient::outbound(&Arc::new(policy)).unwrap()
}

async fn serve_once(listener: &TcpListener, status: &str, body: &str) -> String {
    let (mut stream, _peer) = listener.accept().await.unwrap();
    let mut request = Vec::new();
    loop {
        let mut chunk = [0_u8; 4096];
        let count = stream.read(&mut chunk).await.unwrap();
        assert!(count > 0);
        request.extend_from_slice(&chunk[..count]);
        let text = String::from_utf8(request.clone()).unwrap();
        if let Some((head, sent)) = text.split_once("\r\n\r\n") {
            let length = head
                .to_ascii_lowercase()
                .lines()
                .find_map(|line| line.strip_prefix("content-length: ")?.parse::<usize>().ok())
                .unwrap_or(0);
            if sent.len() >= length {
                break;
            }
        }
    }
    let reply = format!(
        "HTTP/1.1 {status}\r\nx-hub: 1\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(reply.as_bytes()).await.unwrap();
    String::from_utf8(request).unwrap()
}

async fn predibase_stream(
    status: &str,
) -> (
    String,
    Result<Option<HubCompletion>, HubDispatchError>,
    LoggingState,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let client = client();
    let pooled = PooledPost { client: &client };
    let services = Services::idle();
    let globals = empty_globals();
    let env = environment(&[("PREDIBASE_API_KEY", "pb-env")]);
    let deps = DispatchDeps {
        post: &pooled,
        ..services.deps(&globals, &env)
    };
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let (mut state, log_env) = logging(&redactor);
    let mut log = CallLogging {
        state: &mut state,
        env: &log_env,
    };
    let mut messages = vec![user_message()];
    let mut options = object(&[
        ("stream", PromptJson::Bool(true)),
        ("tenant_id", PromptJson::Text(String::from("tenant-1"))),
    ]);
    let mut call = ChatContext {
        acompletion: true,
        ..chat_context("m", &mut messages, &mut options, Some(base.as_str()), None)
    };
    let selected = selection(ChatConfigTag::Predibase);
    let (request, outcome) = timeout(EXCHANGE_TIMEOUT, async {
        tokio::join!(
            serve_once(&listener, status, "boom"),
            dispatch(&selected, &mut call, &deps, &mut log)
        )
    })
    .await
    .unwrap();
    (request, outcome, state)
}

#[tokio::test]
async fn the_pooled_hub_returns_the_status_headers_and_body() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/api/models/m", listener.local_addr().unwrap());
    let client = client();
    let hub = PooledHub {
        client: &client,
        deadline: EXCHANGE_TIMEOUT,
    };
    let headers = Headers::from([(String::from("Accept"), String::from("application/json"))]);
    let query = [("expand", "inferenceProviderMapping")];
    let (request, answer) = timeout(EXCHANGE_TIMEOUT, async {
        tokio::join!(
            serve_once(&listener, "404 Not Found", r#"{"error":"missing"}"#),
            hub.get(&url, &headers, &query)
        )
    })
    .await
    .unwrap();
    let response = answer.unwrap();
    assert_eq!(response.status_code, 404);
    assert_eq!(response.headers.get("x-hub").map(String::as_str), Some("1"));
    assert_eq!(response.body, br#"{"error":"missing"}"#);
    let line = "GET /api/models/m?expand=inferenceProviderMapping HTTP/1.1\r\n";
    assert!(request.starts_with(line));
    assert!(
        request
            .to_ascii_lowercase()
            .contains("accept: application/json")
    );
}

#[tokio::test]
async fn the_predibase_stream_fails_on_a_status_other_than_200() {
    let (request, outcome, state) = predibase_stream("500 Internal Server Error").await;
    let Err(HubDispatchError::Predibase(error)) = outcome else {
        panic!("a failed stream answer is a Predibase error");
    };
    assert_eq!(error.status_code, 500);
    let line = "POST /tenant-1/deployments/v2/llms/m/generate_stream HTTP/1.1\r\n";
    assert!(request.starts_with(line));
    assert!(request.contains(r#""stream":true"#));
    let event = state.model_call_details.get("log_event_type");
    assert_eq!(event, Some(&Json::Text(String::from("pre_api_call"))));
}

#[tokio::test]
async fn the_predibase_stream_logs_the_post_call_on_200() {
    let (_request, outcome, state) = predibase_stream("200 OK").await;
    let Some(HubCompletion::Predibase(completion)) = outcome.unwrap() else {
        panic!("the predibase tag returns a predibase completion");
    };
    assert!(matches!(*completion, predibase::Completion::Stream(_)));
    let event = state.model_call_details.get("log_event_type");
    assert_eq!(event, Some(&Json::Text(String::from("post_api_call"))));
    let original = state.model_call_details.get("original_response");
    assert_eq!(original, Some(&Json::Null));
}

#[tokio::test]
async fn the_predibase_arm_builds_the_prompt_from_the_custom_prompt_dict() {
    let services = Services::new(
        ScriptedHub::new(Vec::new()),
        ScriptedPost::new(vec![http_reply(200, r#"{"generated_text":"hello"}"#)]),
        ScriptedTransport::new(Vec::new()),
    );
    let template: CustomPromptTemplate = serde_json::from_str(TEMPLATE).unwrap();
    let table = CustomPromptTable::from([(String::from(PB_MODEL), template)]);
    let mut messages = vec![user_message()];
    let mut options = object(&[("tenant_id", PromptJson::Text(String::from("tenant-1")))]);
    let mut call = ChatContext {
        custom_prompt_dict: Some(&table),
        ..chat_context(
            PB_MODEL,
            &mut messages,
            &mut options,
            Some("http://mock"),
            None,
        )
    };
    let env = environment(&[("PREDIBASE_API_KEY", "pb-env")]);
    let (outcome, state) =
        dispatch_with(&services, &env, ChatConfigTag::Predibase, &mut call).await;
    outcome.unwrap();
    let requests = services.post.requests();
    let [(_, _, sent_body)] = requests.as_slice() else {
        panic!("the predibase arm posts once");
    };
    let body: Object = serde_json::from_str(sent_body).unwrap();
    let inputs = PromptJson::Text(String::from("IU:hi!F"));
    assert_eq!(body.get("inputs"), Some(&inputs));
    let Some(Json::Object(raw)) = state.model_call_details.get("raw_request_typed_dict") else {
        panic!("the pre_call records the raw request");
    };
    let Some(Json::Object(sent)) = raw.get("raw_request_headers") else {
        panic!("the raw request keeps its headers");
    };
    assert!(sent.contains_key("content-type"));
}

#[test]
fn the_placeholder_sources_never_answer() {
    assert_eq!(NoTag.tag("m", None).unwrap(), None);
    let mapping = NoMapping.mapping("m");
    assert!(matches!(mapping, Err(HuggingfaceChatError::MappingMissing)));
}

#[tokio::test]
async fn the_huggingface_arm_skips_the_mapping_when_a_base_is_set() {
    let services = Services::idle();
    let mut messages = vec![user_message()];
    let mut options = Object::new();
    let model = "hf-inference/meta-llama/Llama-3-8B-Instruct";
    let base = Some("http://mock/v1");
    let mut call = chat_context(model, &mut messages, &mut options, base, None);
    let env = environment(&[]);
    let (outcome, _logged) =
        dispatch_with(&services, &env, ChatConfigTag::HuggingFaceChat, &mut call).await;
    let Some(HubCompletion::HuggingFace(request)) = outcome.unwrap() else {
        panic!("the hugging face tag prepares a request");
    };
    let PreparedRequest::Chat(prepared) = *request else {
        panic!("a plain call prepares a chat request");
    };
    let body: Object = serde_json::from_slice(&prepared.http.body).unwrap();
    let sent_model = PromptJson::Text(String::from(model));
    assert_eq!(body.get("model"), Some(&sent_model));
    assert_eq!(services.hub.calls(), 0);
}

#[test]
fn the_rerank_config_maps_the_cohere_params_and_builds_the_body() {
    let globals = empty_globals();
    let config = huggingface_rerank_config(None, None, &globals, &environment(&[])).unwrap();
    let documents = Json::List(vec![Json::Text(String::from("d"))]);
    let mut cohere = RerankParams::from([
        (String::from("query"), Json::Text(String::from("q"))),
        (String::from("documents"), documents.clone()),
    ]);
    let mapped = config
        .map_cohere_rerank_params(MapCohereRerankParams {
            non_default_params: &mut cohere,
            model: "m",
            drop_params: false,
            query: "q",
            documents: &[],
            custom_llm_provider: None,
            top_n: None,
            rank_fields: None,
            return_documents: None,
            max_chunks_per_doc: None,
            max_tokens_per_doc: None,
            instruction: None,
        })
        .unwrap();
    assert_eq!(mapped.get("texts"), Some(&documents));
    let body = config
        .transform_rerank_request("m", &mapped, &Headers::new(), None)
        .unwrap();
    let direction = Json::Text(String::from("Right"));
    assert_eq!(body.get("truncation_direction"), Some(&direction));
}

#[test]
fn the_rerank_config_reads_the_reply_with_the_module_counter() {
    let globals = empty_globals();
    let config = huggingface_rerank_config(None, None, &globals, &environment(&[])).unwrap();
    let raw = RerankProviderResponse {
        status_code: 200,
        headers: Headers::new(),
        body: br#"[{"index":0,"score":0.5}]"#.to_vec(),
    };
    let request = RerankParams::from([
        (String::from("query"), Json::Text(String::from("q"))),
        (
            String::from("texts"),
            Json::List(vec![Json::Text(String::from("d"))]),
        ),
    ]);
    let empty = RerankParams::new();
    let mut logging = RerankLogging::default();
    let transform = ResponseTransform {
        model: "m",
        raw_response: &raw,
        logging: &mut logging,
        api_key: None,
        request_data: &request,
        optional_params: &empty,
        litellm_params: &empty,
    };
    let response = config
        .transform_rerank_response(RerankResponse::default(), transform)
        .unwrap();
    assert!(response.id.is_some());
    assert_eq!(response.results.map(|results| results.len()), Some(1));
}
