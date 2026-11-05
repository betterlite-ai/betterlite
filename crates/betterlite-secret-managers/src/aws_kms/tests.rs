#![cfg(test)]

use aws_sdk_kms::config::http::{HttpRequest, HttpResponse};
use aws_smithy_runtime_api::client::{
    http::{
        HttpClient, HttpConnector, HttpConnectorFuture, HttpConnectorSettings, SharedHttpConnector,
    },
    runtime_components::RuntimeComponents,
};
use aws_smithy_types::body::SdkBody;
use http::Response;
use std::{
    future::pending,
    sync::{Arc, Mutex},
};

#[derive(Clone, Debug)]
pub(super) struct Replay {
    pub(super) response: Arc<Mutex<Option<HttpResponse>>>,
}

impl HttpConnector for Replay {
    fn call(&self, request: HttpRequest) -> HttpConnectorFuture {
        assert_eq!(request.method(), "POST");
        assert_eq!(request.uri(), "https://kms.us-east-1.amazonaws.com/");
        assert_eq!(
            request.body().bytes(),
            Some(br#"{"CiphertextBlob":"aGVsbG8="}"#.as_slice())
        );
        let response = self.response.lock().unwrap().take();
        HttpConnectorFuture::new(async move {
            match response {
                Some(value) => Ok(value),
                None => pending().await,
            }
        })
    }
}

impl HttpClient for Replay {
    fn http_connector(
        &self,
        _: &HttpConnectorSettings,
        _: &RuntimeComponents,
    ) -> SharedHttpConnector {
        SharedHttpConnector::new(self.clone())
    }
}

pub(super) fn replay(body: &str, status: u16) -> Replay {
    let response = HttpResponse::try_from(
        Response::builder()
            .status(status)
            .body(SdkBody::from(body))
            .unwrap(),
    )
    .unwrap();
    Replay {
        response: Arc::new(Mutex::new(Some(response))),
    }
}
