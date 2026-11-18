#![cfg(test)]

use std::net::TcpListener as StdListener;
use std::thread;

use betterlite_openai::compatible_line_stream::Mode;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use futures_util::future::join;
use tokio::net::TcpListener;
use tokio::runtime::Builder;
use tokio::time::timeout;

use super::super::send::CompatibleChatOutcome;
use super::{Call, EXCHANGE_TIMEOUT, REPLY, client, logging, serve};

// the exchange completes on a thread that is not the one that built it
#[test]
fn the_execution_completes_on_a_second_thread() {
    let std_listener = StdListener::bind("127.0.0.1:0").unwrap();
    std_listener.set_nonblocking(true).unwrap();
    let addr = std_listener.local_addr().unwrap();
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let (mut state, env) = logging(&redactor);
    let mut call = Call::new(Mode::Async);
    call.api_base = Some(format!("http://{addr}/v1"));
    let client = client();
    let execution = call.execution(None, None);
    let future = execution.execute(&client, &mut state, &env);
    let (request, answer) = thread::scope(|scope| {
        scope
            .spawn(move || {
                let runtime = Builder::new_current_thread().enable_all().build().unwrap();
                runtime.block_on(async move {
                    let listener = TcpListener::from_std(std_listener).unwrap();
                    let served = join(serve(&listener, 200, REPLY.as_bytes()), future);
                    timeout(EXCHANGE_TIMEOUT, served).await.unwrap()
                })
            })
            .join()
            .unwrap()
    });
    let CompatibleChatOutcome::Reply(reply) = answer.unwrap() else {
        panic!("a buffered reply");
    };
    assert_eq!(reply.response.model, "openai_like/gpt-4o");
    assert!(request.starts_with("POST /v1/chat/completions HTTP/1.1"));
}
