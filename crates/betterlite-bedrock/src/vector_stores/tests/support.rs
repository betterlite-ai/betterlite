use std::collections::BTreeMap;
use std::future::ready;
use std::io;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde_json::value::RawValue;
use tokio::runtime::{Builder, Runtime};
use tracing::subscriber::{DefaultGuard, set_default};

use crate::aws_settings::sources::Credentials;
use crate::signing::request::AwsSignedRequest;
use crate::signing::target::RegionInputs;
use crate::vector_stores::error::VectorStoreError;
use crate::vector_stores::request::SearchOptionalParams;
use crate::vector_stores::signing::{SignRequest, sign_request};

pub(super) fn json(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

pub(super) fn object(text: &str) -> IndexMap<String, Json> {
    let Json::Object(fields) = json(text) else {
        panic!("the fixture is not an object");
    };
    fields
}

pub(super) fn runtime() -> Runtime {
    Builder::new_current_thread().enable_all().build().unwrap()
}

pub(super) fn credentials() -> Credentials {
    Credentials {
        access_key_id: SecretString::from("AKIAFIXTUREACCESS0001"),
        secret_access_key: SecretString::from("fixture-secret-access-key"),
        session_token: None,
    }
}

pub(super) fn now() -> SystemTime {
    SystemTime::UNIX_EPOCH
        .checked_add(Duration::from_secs(1_700_000_000))
        .unwrap()
}

pub(super) fn optional(
    filters: Option<Json>,
    max_num_results: Option<i64>,
) -> SearchOptionalParams {
    SearchOptionalParams {
        filters,
        max_num_results,
        ranking_options: None,
        rewrite_query: None,
    }
}

pub(super) fn optional_params(pairs: &[(&str, &str)]) -> BTreeMap<String, Box<RawValue>> {
    pairs
        .iter()
        .map(|(name, value)| {
            (
                (*name).to_owned(),
                RawValue::from_string((*value).to_owned()).unwrap(),
            )
        })
        .collect()
}

pub(super) fn sign(
    headers: &[(String, Option<String>)],
    params: &BTreeMap<String, Box<RawValue>>,
    data: &IndexMap<String, Json>,
    api_key: Option<&SecretString>,
) -> Result<AwsSignedRequest, VectorStoreError> {
    runtime().block_on(sign_request(
        SignRequest {
            headers,
            optional_params: params,
            request_data: data,
            api_base: "https://bedrock-agent-runtime.us-east-1.amazonaws.com/knowledgebases",
            api_key,
            configured_bearer: None,
            region_inputs: RegionInputs::default(),
            now: now(),
        },
        |_settings, _region| ready(Ok::<Credentials, _>(credentials())),
    ))
}

#[derive(Clone, Default)]
pub(super) struct Captured(Arc<Mutex<Vec<u8>>>);

impl Captured {
    pub(super) fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

impl io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|error| io::Error::other(error.to_string()))?
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(super) fn capture() -> (Captured, DefaultGuard) {
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_max_level(tracing::Level::DEBUG)
        .with_ansi(false)
        .finish();
    (captured, set_default(subscriber))
}

pub(super) fn header<'signed>(signed: &'signed AwsSignedRequest, name: &str) -> &'signed str {
    signed
        .headers
        .iter()
        .find(|(found, _)| found.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
        .unwrap()
}
