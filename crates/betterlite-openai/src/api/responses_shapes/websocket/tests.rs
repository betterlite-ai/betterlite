#![cfg(test)]

use serde_json::value::RawValue;
use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::io::Error;
use std::sync::Mutex;

use super::{
    PresidioGuardrailCallback, ResponsesBackendWebSocket, ResponsesClientWebSocket,
    ResponsesWebSocketMessage,
};

struct Socket {
    sent: Mutex<Vec<String>>,
}

impl ResponsesClientWebSocket for Socket {
    type Error = Error;

    fn send_text(&self, data: &str) -> impl Future<Output = Result<(), Self::Error>> {
        self.sent.lock().unwrap().push(data.to_owned());
        ready(Ok(()))
    }

    fn receive_text(&self) -> impl Future<Output = Result<String, Self::Error>> {
        ready(Ok("client".to_owned()))
    }
}

impl ResponsesBackendWebSocket for Socket {
    type Error = Error;

    fn recv(
        &self,
        decode: bool,
    ) -> impl Future<Output = Result<ResponsesWebSocketMessage, Self::Error>> {
        ready(Ok(if decode {
            ResponsesWebSocketMessage::Text("upstream".to_owned())
        } else {
            ResponsesWebSocketMessage::Bytes(vec![0, 255])
        }))
    }

    fn send(&self, message: &str) -> impl Future<Output = Result<(), Self::Error>> {
        self.sent.lock().unwrap().push(message.to_owned());
        ready(Ok(()))
    }

    fn close(&self) -> impl Future<Output = Result<(), Self::Error>> {
        ready(Err(Error::other("closed")))
    }
}

impl PresidioGuardrailCallback for Socket {
    type Error = Error;
    type Settings = String;

    fn presidio_settings(&self, data: &BTreeMap<String, Box<RawValue>>) -> Option<Self::Settings> {
        data.get("language").map(|raw| raw.get().to_owned())
    }

    fn check_pii(
        &self,
        text: &str,
        output_parse_pii: bool,
        presidio_config: Option<&Self::Settings>,
        request_data: &BTreeMap<String, Box<RawValue>>,
    ) -> impl Future<Output = Result<String, Self::Error>> {
        assert!(output_parse_pii);
        assert_eq!(
            presidio_config,
            self.presidio_settings(request_data).as_ref()
        );
        ready(Ok(text.replace("private", "masked")))
    }
}

#[tokio::test]
async fn injected_protocols_preserve_values_and_errors() {
    let socket = Socket {
        sent: Mutex::new(Vec::new()),
    };
    socket.send_text("client frame").await.unwrap();
    socket.send("upstream frame").await.unwrap();
    assert_eq!(
        *socket.sent.lock().unwrap(),
        ["client frame", "upstream frame"]
    );
    assert_eq!(socket.receive_text().await.unwrap(), "client");
    assert_eq!(
        socket.recv(true).await.unwrap(),
        ResponsesWebSocketMessage::Text("upstream".to_owned())
    );
    assert_eq!(
        socket.recv(false).await.unwrap(),
        ResponsesWebSocketMessage::Bytes(vec![0, 255])
    );
    assert_eq!(socket.close().await.unwrap_err().to_string(), "closed");
    let request = serde_json::from_str(r#"{"language":"en"}"#).unwrap();
    let config = socket.presidio_settings(&request);
    assert_eq!(
        socket
            .check_pii("private", true, config.as_ref(), &request)
            .await
            .unwrap(),
        "masked"
    );
}
