use std::io;
use std::time::Duration;

use http::Method;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use url::Url;

use crate::agent_launch::KeyProbe;
use crate::autoroute::config::model::RawModelListing;
use crate::autoroute::wizard::ModelDiscoveryClient;
use crate::autoroute::wizard::error::ModelDiscoveryError;
use crate::commands::{Client, ClientError, Payload, Request};
use crate::opencode::{ModelListingRequest, ModelListingResponse, ModelListingTransport};
use crate::pi::{ProxyGet, ProxyResponse};
use crate::pkce_login::HttpResponse;
use crate::sso_polling::transport::PollTransport;

fn send(
    client: &Client,
    url: &str,
    method: Method,
    headers: Vec<(String, String)>,
) -> io::Result<HttpResponse> {
    let parsed =
        Url::parse(url).map_err(|source| io::Error::new(io::ErrorKind::InvalidInput, source))?;
    client
        .send(Request {
            method,
            url: parsed,
            headers,
            payload: Payload::Empty,
        })
        .map_err(io::Error::other)
}

fn exposed(headers: &[(String, SecretString)]) -> Vec<(String, String)> {
    headers
        .iter()
        .map(|(name, value)| (name.clone(), value.expose_secret().to_owned()))
        .collect()
}

impl PollTransport for Client {
    fn get(
        &self,
        url: &str,
        headers: &[(String, SecretString)],
        _timeout_seconds: u64,
    ) -> io::Result<HttpResponse> {
        send(self, url, Method::GET, exposed(headers))
    }

    fn post(&self, url: &str, _timeout_seconds: u64) -> io::Result<HttpResponse> {
        send(self, url, Method::POST, Vec::new())
    }
}

impl KeyProbe for Client {
    fn status(&self, url: &str, api_key: &SecretString, _timeout: Duration) -> io::Result<u16> {
        let headers = vec![(
            String::from("Authorization"),
            format!("Bearer {}", api_key.expose_secret()),
        )];
        Ok(send(self, url, Method::GET, headers)?.status)
    }
}

impl ModelListingTransport for Client {
    fn get(&self, request: &ModelListingRequest) -> io::Result<ModelListingResponse> {
        let headers = vec![(
            String::from("Authorization"),
            request.authorization.expose_secret().to_owned(),
        )];
        let response = send(self, &request.url, Method::GET, headers)?;
        Ok(ModelListingResponse {
            status: response.status,
            body: response.body,
        })
    }
}

impl ProxyGet for Client {
    fn get(
        &self,
        url: &str,
        headers: &[(String, SecretString)],
        _timeout: Duration,
    ) -> io::Result<ProxyResponse> {
        let response = send(self, url, Method::GET, exposed(headers))?;
        Ok(ProxyResponse {
            status: response.status,
            body: response.body,
        })
    }
}

#[derive(Deserialize)]
struct ModelListing {
    data: Vec<RawModelListing>,
}

fn discovery_failure(source: ClientError) -> ModelDiscoveryError {
    ModelDiscoveryError::NotAList(serde_json::Error::io(io::Error::other(source)))
}

impl ModelDiscoveryClient for Client {
    fn list_models(&self) -> Result<Vec<RawModelListing>, ModelDiscoveryError> {
        let url = self.resolve("models").map_err(discovery_failure)?;
        let response = self
            .send(Request {
                method: Method::GET,
                url,
                headers: Vec::new(),
                payload: Payload::Empty,
            })
            .map_err(discovery_failure)?;
        serde_json::from_slice::<ModelListing>(&response.body)
            .map(|listing| listing.data)
            .map_err(ModelDiscoveryError::NotAList)
    }
}
