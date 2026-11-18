#![cfg(test)]

use std::future::Future;
use std::net::TcpListener as StdListener;
use std::thread;

use betterlite_config::sections::compatible_embedding_settings::Globals;
use betterlite_models::decisions::Json;
use betterlite_openai::compatible_embedding::Mode;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use futures_util::future::join;
use indexmap::IndexMap;
use secrecy::SecretString;
use tokio::net::TcpListener;
use tokio::runtime::Builder;
use tokio::time::timeout;

use super::super::{SharedEmbedding, shared_embedding};
use super::{EXCHANGE_TIMEOUT, client, logging_fixture, serve, settings};

// serves one request against the call's future on a thread of its own and gives back both results
fn exchange<Output, Call>(std_listener: StdListener, future: Call) -> (String, Output)
where
    Output: Send,
    Call: Future<Output = Output> + Send,
{
    thread::scope(|scope| {
        scope
            .spawn(move || {
                let runtime = Builder::new_current_thread().enable_all().build().unwrap();
                runtime.block_on(async move {
                    let listener = TcpListener::from_std(std_listener).unwrap();
                    let served = join(serve(&listener), future);
                    timeout(EXCHANGE_TIMEOUT, served).await.unwrap()
                })
            })
            .join()
            .unwrap()
    })
}

// the shared handler completes on a thread that is not the one that built it
#[test]
fn shared_embedding_completes_on_a_second_thread() {
    let std_listener = StdListener::bind("127.0.0.1:0").unwrap();
    std_listener.set_nonblocking(true).unwrap();
    let base = format!("http://{}/v1", std_listener.local_addr().unwrap());
    let (settings, key) = (
        settings(&[]),
        SecretString::from(String::from("sk-explicit")),
    );
    let embedding = SharedEmbedding {
        model: "m",
        input: &Json::List(vec![Json::Text(String::from("a"))]),
        api_base: Some(&base),
        api_key: Some(&key),
        optional_params: &IndexMap::new(),
        headers: None,
        custom_endpoint: false,
        timeout: EXCHANGE_TIMEOUT,
        mode: Mode::Async,
        globals: &Globals::default(),
        settings: &settings,
    };
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let (mut state, env) = logging_fixture(&redactor);
    let client = client();
    let (request, answer) = exchange(
        std_listener,
        shared_embedding(&embedding, &client, &mut state, &env),
    );
    let response = answer.unwrap();
    assert_eq!(response.model, "upstream");
    assert_eq!(response.data.len(), 1);
    assert!(request.starts_with("POST /v1/embeddings HTTP/1.1"));
}
