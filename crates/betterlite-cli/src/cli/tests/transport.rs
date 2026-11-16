use std::thread;
use std::time::Duration;

use secrecy::SecretString;

use crate::agent_launch::KeyProbe;
use crate::autoroute::wizard::ModelDiscoveryClient;
use crate::cli::tests::support::{Answer, FakeEnvironment, serve};
use crate::cli_context::CliContext;
use crate::commands::Client;
use crate::opencode::{ModelListingRequest, ModelListingTransport};
use crate::pi::ProxyGet;
use crate::sso_polling::transport::PollTransport;

const MODELS_BODY: &str = "{\"data\":[{\"id\":\"m1\"},{\"id\":\"m2\",\"mode\":\"embedding\"}]}";

fn client(base_url: &str) -> Client {
    let context = CliContext {
        base_url: base_url.to_owned(),
        base_url_explicit: true,
        api_key: None,
        api_key_from_token_file: false,
    };
    Client::new(&context, &FakeEnvironment::default()).unwrap()
}

fn answer(body: &'static str) -> Answer {
    Answer {
        status: 200,
        reason: "OK",
        body,
    }
}

#[test]
fn polling_transport_sends_exposed_headers_and_keeps_the_body() {
    thread::scope(|scope| {
        let served = serve(scope, answer("polled"));
        let client = client(&served.base_url);
        let headers = vec![(
            String::from("Authorization"),
            SecretString::from("poll-key"),
        )];
        let url = format!("{}/sso/poll", served.base_url);
        let response = PollTransport::get(&client, &url, &headers, 2).unwrap();
        assert_eq!(response.status, 200);
        assert_eq!(response.body, b"polled".to_vec());
        assert!(
            served
                .request()
                .to_lowercase()
                .contains("authorization: poll-key")
        );
    });
}

#[test]
fn polling_transport_posts_and_keeps_the_body() {
    thread::scope(|scope| {
        let served = serve(scope, answer("posted"));
        let client = client(&served.base_url);
        let url = format!("{}/sso/exchange", served.base_url);
        let response = PollTransport::post(&client, &url, 2).unwrap();
        assert_eq!(response.status, 200);
        assert_eq!(response.body, b"posted".to_vec());
        assert!(served.request().starts_with("POST /sso/exchange"));
    });
}

#[test]
fn proxy_get_sends_exposed_headers_and_returns_status_and_body() {
    thread::scope(|scope| {
        let served = serve(scope, answer("proxied"));
        let client = client(&served.base_url);
        let headers = vec![(String::from("X-Extra"), SecretString::from("proxy-secret"))];
        let url = format!("{}/v1/models", served.base_url);
        let response = ProxyGet::get(&client, &url, &headers, Duration::from_secs(2)).unwrap();
        assert_eq!(response.status, 200);
        assert_eq!(response.body, b"proxied".to_vec());
        assert!(
            served
                .request()
                .to_lowercase()
                .contains("x-extra: proxy-secret")
        );
    });
}

#[test]
fn key_probe_returns_the_answered_status() {
    thread::scope(|scope| {
        let served = serve(
            scope,
            Answer {
                status: 204,
                reason: "No Content",
                body: "",
            },
        );
        let client = client(&served.base_url);
        let url = format!("{}/health", served.base_url);
        let status = KeyProbe::status(
            &client,
            &url,
            &SecretString::from("probe-key"),
            Duration::from_secs(2),
        )
        .unwrap();
        assert_eq!(status, 204);
        assert!(
            served
                .request()
                .to_lowercase()
                .contains("authorization: bearer probe-key")
        );
    });
}

#[test]
fn model_listing_transport_returns_the_status_and_body() {
    thread::scope(|scope| {
        let served = serve(scope, answer(MODELS_BODY));
        let client = client(&served.base_url);
        let request = ModelListingRequest {
            url: format!("{}/v1/models", served.base_url),
            authorization: SecretString::from("Bearer listing-key"),
            timeout: Duration::from_secs(2),
            max_body_bytes: 4096,
        };
        let response = ModelListingTransport::get(&client, &request).unwrap();
        assert_eq!(response.status, 200);
        assert_eq!(response.body, MODELS_BODY.as_bytes().to_vec());
        assert!(
            served
                .request()
                .to_lowercase()
                .contains("authorization: bearer listing-key")
        );
    });
}

#[test]
fn model_discovery_reads_every_reported_model() {
    thread::scope(|scope| {
        let served = serve(scope, answer(MODELS_BODY));
        let client = client(&served.base_url);
        let models = ModelDiscoveryClient::list_models(&client).unwrap();
        let ids: Vec<&str> = models.iter().map(|model| model.id.as_str()).collect();
        assert_eq!(ids, ["m1", "m2"]);
        assert!(served.request().contains("/models"));
    });
}
