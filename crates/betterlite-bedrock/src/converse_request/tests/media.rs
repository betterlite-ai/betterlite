use std::io::{self, Read as _, Write as _};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::str;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use betterlite_prompts::bedrock_processor::error::ProcessorError;

use super::support::{
    CLAUDE_3_5, Call, Refusing, expected, field, processor_error, remote, remote_for, run,
};

const PNG: &[u8] = b"\x89PNG\r\n\x1a\n";

struct Server {
    address: SocketAddr,
    requests: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
}

impl Server {
    fn start<'scope>(scope: &'scope thread::Scope<'scope, '_>, body: &'static [u8]) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("the fixture listener binds");
        let address = listener.local_addr().expect("the fixture address reads");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let recorded = Arc::clone(&requests);
        let halted = Arc::clone(&stop);
        scope.spawn(move || {
            while !halted.load(Ordering::SeqCst) {
                let Ok((stream, _)) = listener.accept() else {
                    return;
                };
                serve(stream, &recorded, body);
            }
        });
        Self {
            address,
            requests,
            stop,
        }
    }

    fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.address)
    }

    fn request_count(&self) -> usize {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // A blocking accept returns only for a connection, so the drop connects once to wake it.
        let _wake = TcpStream::connect(self.address);
    }
}

fn serve(mut stream: TcpStream, requests: &Mutex<Vec<String>>, body: &[u8]) {
    stream
        .set_nonblocking(false)
        .expect("the accepted stream blocks");
    let mut head = Vec::new();
    let mut chunk = [0_u8; 1024];
    while !head.windows(4).any(|window| window == b"\r\n\r\n") {
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => return,
            Ok(count) => {
                let Some(received) = chunk.get(..count) else {
                    return;
                };
                head.extend_from_slice(received);
            }
        }
    }
    let path = str::from_utf8(&head)
        .ok()
        .and_then(|text| text.lines().next())
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or_default()
        .to_owned();
    requests
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(path);
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _written = stream
        .write_all(response.as_bytes())
        .and_then(|()| stream.write_all(body));
}

fn image_messages(url: &str) -> String {
    format!(r#"[{{"role": "user", "content": [{{"type": "image_url", "image_url": "{url}"}}]}}]"#)
}

fn image_only_body(bytes: &[u8]) -> String {
    format!(
        r#"[{{"role": "user", "content": [{{"image": {{"format": "png", "source": {{"bytes": "{}"}}}}}}]}}]"#,
        BASE64.encode(bytes)
    )
}

// An http image is fetched once through the injected transport; a refusing resolver fails before any connection.
#[test]
fn converse_request_media_fetch() {
    thread::scope(|scope| {
        let server = Server::start(scope, PNG);
        let messages = image_messages(&server.url("/image.png"));
        let call = Call {
            messages: &messages,
            ..Call::new(CLAUDE_3_5, "{}")
        };
        let (outcome, _warnings) = run(&call, &remote(&["127.0.0.1"]));
        assert_eq!(
            field(outcome.body(), "messages"),
            Some(&expected(&image_only_body(PNG)))
        );
        assert_eq!(server.request_count(), 1);

        let unreachable = Server::start(scope, PNG);
        let messages = image_messages(&unreachable.url("/image.png"));
        let call = Call {
            messages: &messages,
            ..Call::new(CLAUDE_3_5, "{}")
        };
        let (refused, _warnings) = run(&call, &remote_for(Refusing, &["127.0.0.1"], 50.0));
        assert!(matches!(
            processor_error(refused.error()),
            Some(ProcessorError::Image { block: 0, .. })
        ));
        assert_eq!(unreachable.request_count(), 0);
    });
}
