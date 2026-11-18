#![cfg(test)]

use std::net::TcpListener as StdListener;
use std::thread;

use betterlite_config::sections::compatible_embedding_settings::Globals;
use betterlite_models::decisions::Json;
use betterlite_openai::compatible_embedding::Mode;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use indexmap::IndexMap;
use secrecy::SecretString;
use tokio::net::TcpListener;
use tokio::runtime::Builder;
use tokio::time::timeout;

use super::super::{SharedEmbedding, shared_embedding};
use super::{EXCHANGE_TIMEOUT, client, logging_fixture, serve, settings};

// the shared handler completes on a thread that is not the one that built it
#[test]
fn shared_embedding_completes_on_a_second_thread() {
    let std_listener = StdListener::bind("127.0.0.1:0").unwrap();
    std_listener.set_nonblocking(true).unwrap();
    let addr = std_listener.local_addr().unwrap();
    let base = format!("http://{addr}/v1");
    let input = Json::List(vec![Json::Text(String::from("a"))]);
    let options = IndexMap::new();
    let globals = Globals::default();
    let settings = settings(&[]);
    let key = SecretString::from(String::from("sk-explicit"));
    let embedding = SharedEmbedding {
        model: "m",
        input: &input,
        api_base: Some(&base),
        api_key: Some(&key),
        optional_params: &options,
        headers: None,
        custom_endpoint: false,
        timeout: EXCHANGE_TIMEOUT,
        mode: Mode::Async,
        globals: &globals,
        settings: &settings,
    };
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let (mut state, env) = logging_fixture(&redactor);
    let client = client();
    let future = shared_embedding(&embedding, &client, &mut state, &env);
    let (request, answer) = thread::scope(|scope| {
        scope
            .spawn(move || {
                let runtime = Builder::new_current_thread().enable_all().build().unwrap();
                runtime.block_on(async move {
                    let listener = TcpListener::from_std(std_listener).unwrap();
                    timeout(EXCHANGE_TIMEOUT, async {
                        tokio::join!(serve(&listener), future)
                    })
                    .await
                    .unwrap()
                })
            })
            .join()
            .unwrap()
    });
    let response = answer.unwrap();
    assert_eq!(response.model, "upstream");
    assert_eq!(response.data.len(), 1);
    assert!(request.starts_with("POST /v1/embeddings HTTP/1.1"));
}
