use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::io;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use betterlite_azure::azure_certificate::CertificateError;
use betterlite_azure::azure_certificate::assertion::RandomIds;
use betterlite_azure::azure_certificate::credential::{
    CertificateCredential, CertificateInput, FileReader,
};
use betterlite_azure::azure_client_secret::transport::{
    Clock, HttpMethod, HttpReply, HttpRequest, Transport,
};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperError, HelperResult};

mod tests;

const FIXTURE_NOW: i64 = 1_700_000_000;
const CERTIFICATE_PATH: &str = "certificate";

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    steps: Vec<Step>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Step {
    #[serde(default = "default_tenant_id")]
    tenant_id: String,
    #[serde(default = "default_client_id")]
    client_id: String,
    #[serde(default)]
    certificate: Option<String>,
    #[serde(default)]
    certificate_base64: Option<String>,
    #[serde(default)]
    missing_file: bool,
    #[serde(default)]
    password: Option<String>,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    send_certificate_chain: bool,
    #[serde(default)]
    replies: Vec<Reply>,
    #[serde(default)]
    advance: Option<i64>,
}

fn default_tenant_id() -> String {
    "tenant".to_owned()
}

fn default_client_id() -> String {
    "client".to_owned()
}

#[derive(Deserialize, Clone)]
#[serde(deny_unknown_fields)]
struct Reply {
    status: u16,
    body: String,
}

struct FixtureTransport {
    replies: Mutex<VecDeque<Reply>>,
    posts: Mutex<Vec<HttpRequest>>,
}

impl FixtureTransport {
    fn posted(&self) -> Vec<HttpRequest> {
        self.posts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl Transport for FixtureTransport {
    fn send(&self, request: &HttpRequest) -> io::Result<HttpReply> {
        if request.method == HttpMethod::Post {
            self.posts
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(request.clone());
        }
        let reply = self
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .ok_or_else(|| io::Error::other(format!("no reply for {}", request.url)))?;
        Ok(HttpReply {
            status: reply.status,
            body: SecretString::from(reply.body),
        })
    }
}

struct FixtureClock(Arc<AtomicI64>);

impl Clock for FixtureClock {
    fn now_unix_seconds(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

struct FixtureFiles {
    bytes: Option<Vec<u8>>,
}

impl FileReader for FixtureFiles {
    fn read(&self, _path: &str) -> io::Result<Vec<u8>> {
        self.bytes
            .clone()
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
    }
}

fn failure(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "InvalidCase".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

// azure-identity lets the OS error escape, so only the kind of failure is compared
fn message(error: &CertificateError) -> String {
    if matches!(error, CertificateError::Read { .. }) {
        return "unreadable".to_owned();
    }
    error.to_string()
}

fn certificate_bytes(step: &Step) -> Result<Option<Vec<u8>>, Box<HelperError>> {
    if step.missing_file {
        return Ok(None);
    }
    match (&step.certificate, &step.certificate_base64) {
        (Some(text), None) => Ok(Some(text.clone().into_bytes())),
        (None, Some(encoded)) => STANDARD
            .decode(encoded)
            .map(Some)
            .map_err(|error| failure(error.to_string())),
        _ => Err(failure(
            "a step needs exactly one of certificate and certificate_base64".to_owned(),
        )),
    }
}

fn segment(assertion: &str, index: usize) -> Value {
    assertion
        .split('.')
        .nth(index)
        .and_then(|text| URL_SAFE_NO_PAD.decode(text).ok())
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or(Value::Null)
}

// the jti is random, so a report says whether it repeats one already sent
fn request_report(post: &HttpRequest, jtis: &mut BTreeSet<String>) -> Value {
    let assertion = post
        .form
        .iter()
        .find(|(name, _value)| name == "client_assertion")
        .map_or("", |(_name, value)| value.as_str());
    let mut claims = segment(assertion, 1);
    let jti = claims
        .get("jti")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_default();
    let label = if jtis.insert(jti) { "new" } else { "repeated" };
    if let Some(object) = claims.as_object_mut() {
        object.insert("jti".to_owned(), json!(label));
    }
    let form: BTreeMap<&str, &str> = post
        .form
        .iter()
        .filter(|(name, _value)| name != "client_assertion")
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect();
    let accept = post
        .headers
        .iter()
        .find(|(name, _value)| name == "Accept")
        .map(|(_name, value)| value.as_str());
    json!({
        "url": post.url,
        "accept": accept,
        "form": form,
        "header": segment(assertion, 0),
        "claims": claims,
    })
}

fn attempt(
    credential: &CertificateCredential,
    transport: &FixtureTransport,
    jtis: &mut BTreeSet<String>,
) -> Value {
    let before = transport.posted().len();
    match credential.token() {
        Err(error) => json!({ "error": message(&error) }),
        Ok(token) => {
            let posts = transport.posted();
            let request = posts
                .get(before)
                .map_or(Value::Null, |post| request_report(post, jtis));
            json!({
                "token": token.token.expose_secret(),
                "expires_on": token.expires_on,
                "token_type": token.token_type,
                "refresh_on": token.refresh_on,
                "request": request,
            })
        }
    }
}

fn run_step(step: &Step) -> HelperResult {
    let files = FixtureFiles {
        bytes: certificate_bytes(step)?,
    };
    let transport = Arc::new(FixtureTransport {
        replies: Mutex::new(VecDeque::from(step.replies.clone())),
        posts: Mutex::new(Vec::new()),
    });
    let now = Arc::new(AtomicI64::new(FIXTURE_NOW));
    let input = CertificateInput {
        tenant_id: step.tenant_id.clone(),
        client_id: step.client_id.clone(),
        scope: step.scope.clone(),
        certificate_path: Some(CERTIFICATE_PATH.to_owned()),
        certificate_data: None,
        password: step.password.clone().map(SecretString::from),
        send_certificate_chain: step.send_certificate_chain,
        authority_host: None,
    };
    let pool = Arc::clone(&transport);
    let ticking = Arc::new(FixtureClock(Arc::clone(&now)));
    let ids = Arc::new(RandomIds);
    let credential = match CertificateCredential::new(input, &files, pool, ticking, ids) {
        Ok(built) => built,
        Err(error) => return Ok(json!([{ "error": message(&error) }])),
    };
    let mut jtis = BTreeSet::new();
    let mut attempts = vec![attempt(&credential, &transport, &mut jtis)];
    if let Some(advance) = step.advance {
        now.store(FIXTURE_NOW.saturating_add(advance), Ordering::SeqCst);
        attempts.push(attempt(&credential, &transport, &mut jtis));
    }
    Ok(Value::Array(attempts))
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let mut reports: Vec<Value> = Vec::with_capacity(input.steps.len());
        for step in &input.steps {
            reports.push(run_step(step)?);
        }
        Ok(Value::Array(reports))
    }
}
