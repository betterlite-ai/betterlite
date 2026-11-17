#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::future::{Future, ready};
use std::io;
use std::mem::take;
use std::path::Path;
use std::sync::Mutex;

use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_prompts::arguments::json::{Json, Object};
use http::Method;
use http::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderName};
use rstest::rstest;
use secrecy::SecretString;
use tokio::runtime::Builder;

use crate::gemini_media::gcs_metadata::{
    GcsDeps, GcsMetadataError, GcsTransport, METADATA_DEADLINE, MetadataFuture, auth_values,
    bucket_is_valid, metadata_url, object_content_type, read_content_type,
};
use crate::token_cache::error::CredentialCacheError;
use crate::token_cache::probe::AccessToken;
use crate::vertex_interactions::mint::{MintAccessToken, MintFuture};

const PNG_REPLY: &str = r#"{"contentType":"image/png"}"#;
const OBJECT: &str = "gs://my-bucket/object";
const OBJECT_METADATA: &str =
    "https://storage.googleapis.com/storage/v1/b/my-bucket/o/object?fields=contentType";
const UNDECODABLE_CREDENTIALS: &str =
    r#"{"vertex_credentials":"{\"type\":\"external_account_authorized_user\"}"}"#;

pub(crate) fn parse_object(source: &str) -> Object {
    serde_json::from_str(source).expect("fixture object")
}

fn block_on<F>(future: F) -> F::Output
where
    F: Future,
{
    Builder::new_current_thread()
        .build()
        .expect("a fixture runtime")
        .block_on(future)
}

#[derive(Default)]
pub(crate) struct ScriptedGcs {
    default: Option<(u16, String)>,
    replies: BTreeMap<String, (u16, String)>,
    requests: Mutex<Vec<OutboundRequest>>,
}

impl ScriptedGcs {
    pub(crate) fn answering(status: u16, body: &str) -> Self {
        Self {
            default: Some((status, body.to_owned())),
            replies: BTreeMap::new(),
            requests: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn failing() -> Self {
        Self::default()
    }

    pub(crate) fn with_reply(&mut self, url: &str, status: u16, body: &str) {
        self.replies
            .insert(url.to_owned(), (status, body.to_owned()));
    }

    pub(crate) fn requested(&self) -> Vec<(String, Option<String>)> {
        self.requests
            .lock()
            .expect("fixture gcs")
            .iter()
            .map(|request| {
                let authorization = header(request, AUTHORIZATION).map(str::to_owned);
                (request.url.to_string(), authorization)
            })
            .collect()
    }

    pub(crate) fn take_requests(&self) -> Vec<OutboundRequest> {
        take(&mut *self.requests.lock().expect("fixture gcs"))
    }
}

impl GcsTransport for ScriptedGcs {
    fn get(&self, request: OutboundRequest) -> MetadataFuture<'_> {
        let reply = self
            .replies
            .get(request.url.as_str())
            .cloned()
            .or_else(|| self.default.clone());
        self.requests.lock().expect("fixture gcs").push(request);
        let answer = reply.map(|(status, body)| UpstreamResponse {
            status,
            headers: HeaderMap::new(),
            body: body.into_bytes(),
        });
        Box::pin(ready(answer.ok_or(TransportError::Closed)))
    }
}

#[derive(Clone, Copy)]
enum MintScript {
    Mints,
    Fails,
    BadToken,
}

impl MintScript {
    const fn recorder(self) -> RecordingMint {
        let token = match self {
            Self::Mints => Some("token-abc"),
            Self::Fails => None,
            Self::BadToken => Some("bad\ntoken"),
        };
        RecordingMint {
            token,
            projects: Mutex::new(Vec::new()),
        }
    }
}

pub(crate) struct RecordingMint {
    token: Option<&'static str>,
    projects: Mutex<Vec<Option<String>>>,
}

impl Default for RecordingMint {
    fn default() -> Self {
        MintScript::Mints.recorder()
    }
}

impl RecordingMint {
    pub(crate) const fn failing() -> Self {
        MintScript::Fails.recorder()
    }

    pub(crate) fn projects(&self) -> Vec<Option<String>> {
        self.projects.lock().expect("fixture mint").clone()
    }
}

impl MintAccessToken for RecordingMint {
    fn mint<'mint>(
        &'mint self,
        _credentials: Option<&'mint ConfiguredCredentials>,
        project: Option<&'mint str>,
    ) -> MintFuture<'mint> {
        self.projects
            .lock()
            .expect("fixture mint")
            .push(project.map(str::to_owned));
        let Some(token) = self.token else {
            return Box::pin(ready(Err(CredentialCacheError::MissingCredentials)));
        };
        Box::pin(ready(Ok(AccessToken {
            token: SecretString::from(token),
            project: project.unwrap_or_default().to_owned(),
        })))
    }
}

fn failure_label(error: &GcsMetadataError) -> String {
    match error {
        GcsMetadataError::Project => "project".to_owned(),
        GcsMetadataError::Credentials => "credentials".to_owned(),
        GcsMetadataError::DecodeCredentials(_) => "decode_credentials".to_owned(),
        GcsMetadataError::WriteCredentials(_) => "write_credentials".to_owned(),
        GcsMetadataError::Mint(_) => "mint".to_owned(),
        GcsMetadataError::Token(_) => "token".to_owned(),
        GcsMetadataError::Transport(_) => "transport".to_owned(),
        GcsMetadataError::Status(status) => format!("status {status}"),
        GcsMetadataError::Decode(_) => "decode".to_owned(),
        GcsMetadataError::NotAnObject => "not_an_object".to_owned(),
        GcsMetadataError::MissingContentType => "missing_content_type".to_owned(),
    }
}

fn describe(reply: &Result<Option<String>, GcsMetadataError>) -> String {
    match reply {
        Ok(Some(value)) => format!("ok {value}"),
        Ok(None) => "ok none".to_owned(),
        Err(failure) => failure_label(failure),
    }
}

fn header(request: &OutboundRequest, name: HeaderName) -> Option<&str> {
    request.headers.get(name)?.to_str().ok()
}

#[rstest]
#[case::plain(
    "gs://my-bucket/object",
    Some("https://storage.googleapis.com/storage/v1/b/my-bucket/o/object?fields=contentType")
)]
#[case::nested(
    "gs://my-bucket/folder/my photo",
    Some(
        "https://storage.googleapis.com/storage/v1/b/my-bucket/o/folder%2Fmy%20photo?fields=contentType"
    )
)]
#[case::percent(
    "gs://my-bucket/100%",
    Some("https://storage.googleapis.com/storage/v1/b/my-bucket/o/100%25?fields=contentType")
)]
#[case::reserved(
    "gs://my-bucket/a b?c#d",
    Some(
        "https://storage.googleapis.com/storage/v1/b/my-bucket/o/a%20b%3Fc%23d?fields=contentType"
    )
)]
#[case::dotted_bucket(
    "gs://my.dotted.bucket/x",
    Some("https://storage.googleapis.com/storage/v1/b/my.dotted.bucket/o/x?fields=contentType")
)]
#[case::shortest_bucket(
    "gs://abc/x",
    Some("https://storage.googleapis.com/storage/v1/b/abc/o/x?fields=contentType")
)]
#[case::dots_in_object(
    "gs://my-bucket/a.b.c",
    Some("https://storage.googleapis.com/storage/v1/b/my-bucket/o/a.b.c?fields=contentType")
)]
#[case::unreserved(
    "gs://my-bucket/-._~AZaz09",
    Some("https://storage.googleapis.com/storage/v1/b/my-bucket/o/-._~AZaz09?fields=contentType")
)]
#[case::unicode(
    "gs://my-bucket/caf\u{e9}",
    Some("https://storage.googleapis.com/storage/v1/b/my-bucket/o/caf%C3%A9?fields=contentType")
)]
#[case::short_bucket("gs://ab/thing", None)]
#[case::uppercase_bucket("gs://My-Bucket/thing", None)]
#[case::leading_dot("gs://.bucket/thing", None)]
#[case::trailing_dot("gs://bucket./thing", None)]
#[case::double_dot("gs://my..bucket/thing", None)]
#[case::dotted_quad("gs://1.2.3.4/thing", None)]
#[case::empty_object("gs://my-bucket/", None)]
#[case::no_slash("gs://my-bucket", None)]
#[case::empty_bucket("gs:///thing", None)]
#[case::not_gs("https://my-bucket/thing", None)]
#[case::no_scheme("my-bucket/thing", None)]
fn gcs_metadata_urls(#[case] uri: &str, #[case] expected: Option<&str>) {
    let url = metadata_url(uri).map(|url| url.to_string());
    assert_eq!(url.as_deref(), expected);
}

#[test]
fn gcs_metadata_bucket_lengths() {
    assert!(bucket_is_valid("abc"));
    assert!(bucket_is_valid("a.b"));
    assert!(!bucket_is_valid("ab"));
    assert!(bucket_is_valid(&"a".repeat(63)));
    assert!(!bucket_is_valid(&"a".repeat(64)));
    assert!(bucket_is_valid(&format!("{}.b", "a".repeat(62))));
    assert!(!bucket_is_valid(&format!("{}.b", "a".repeat(64))));
}

#[test]
fn gcs_metadata_dotted_bucket_lengths() {
    let longest_dotted = format!(
        "{}.{}.{}.{}",
        "a".repeat(63),
        "a".repeat(63),
        "a".repeat(63),
        "a".repeat(30)
    );
    assert_eq!(longest_dotted.len(), 222);
    assert!(bucket_is_valid(&longest_dotted));
    let too_long_dotted = format!(
        "{}.{}.{}.{}",
        "a".repeat(63),
        "a".repeat(63),
        "a".repeat(63),
        "a".repeat(31)
    );
    assert_eq!(too_long_dotted.len(), 223);
    assert!(!bucket_is_valid(&too_long_dotted));
}

#[test]
fn gcs_metadata_bucket_labels() {
    assert!(bucket_is_valid("1.2.3.4.5"));
    assert!(bucket_is_valid("1.2.3.a"));
    assert!(!bucket_is_valid("1.2.3.4"));
    assert!(!bucket_is_valid("my-bucket-"));
    assert!(!bucket_is_valid("my bucket"));
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Present {
    Nothing,
    Project,
    Credentials,
    Both,
}

#[rstest]
#[case::absent("{}", None, None, Present::Nothing)]
#[case::project(r#"{"vertex_project":"p"}"#, Some("p"), None, Present::Project)]
#[case::project_alias(r#"{"vertex_ai_project":"p"}"#, Some("p"), None, Present::Project)]
#[case::empty_project_falls_through(
    r#"{"vertex_project":"","vertex_ai_project":"p"}"#,
    Some("p"),
    None,
    Present::Project
)]
#[case::null_project_falls_through(
    r#"{"vertex_project":null,"vertex_ai_project":"p"}"#,
    Some("p"),
    None,
    Present::Project
)]
#[case::first_project_wins(
    r#"{"vertex_project":"a","vertex_ai_project":"b"}"#,
    Some("a"),
    None,
    Present::Project
)]
#[case::false_project_falls_through(
    r#"{"vertex_project":false,"vertex_ai_project":"p"}"#,
    Some("p"),
    None,
    Present::Project
)]
#[case::zero_project_is_absent(r#"{"vertex_project":0}"#, None, None, Present::Nothing)]
#[case::empty_list_project_is_absent(r#"{"vertex_project":[]}"#, None, None, Present::Nothing)]
#[case::empty_object_project_is_absent(r#"{"vertex_project":{}}"#, None, None, Present::Nothing)]
#[case::credentials(
    r#"{"vertex_credentials":"sa.json"}"#,
    None,
    Some("sa.json"),
    Present::Credentials
)]
#[case::credentials_alias(
    r#"{"vertex_ai_credentials":"sa.json"}"#,
    None,
    Some("sa.json"),
    Present::Credentials
)]
#[case::empty_credentials_fall_through(
    r#"{"vertex_credentials":"","vertex_ai_credentials":"alias.json"}"#,
    None,
    Some("alias.json"),
    Present::Credentials
)]
#[case::both(
    r#"{"vertex_project":"p","vertex_credentials":"c.json"}"#,
    Some("p"),
    Some("c.json"),
    Present::Both
)]
#[case::object_credentials(
    r#"{"vertex_credentials":{"type":"service_account"}}"#,
    None,
    None,
    Present::Credentials
)]
#[case::numeric_project(r#"{"vertex_project":5}"#, None, None, Present::Project)]
fn gcs_metadata_auth_aliases(
    #[case] params: &str,
    #[case] project: Option<&str>,
    #[case] credentials: Option<&str>,
    #[case] present: Present,
) {
    let values = parse_object(params);
    let auth = auth_values(&values);
    assert_eq!(auth.project.and_then(Json::as_str), project);
    assert_eq!(auth.credentials.and_then(Json::as_str), credentials);
    let seen = match (auth.project.is_some(), auth.credentials.is_some()) {
        (false, false) => Present::Nothing,
        (true, false) => Present::Project,
        (false, true) => Present::Credentials,
        (true, true) => Present::Both,
    };
    assert_eq!(seen, present);
}

const AUTHORIZED: &str = r#"{"vertex_project":"p"}"#;

#[rstest]
#[case::authorized(200, PNG_REPLY, AUTHORIZED, MintScript::Mints, "ok image/png")]
#[case::anonymous(200, PNG_REPLY, "{}", MintScript::Mints, "ok image/png")]
#[case::empty_object_authorized(200, "{}", AUTHORIZED, MintScript::Mints, "missing_content_type")]
#[case::empty_object_anonymous(200, "{}", "{}", MintScript::Mints, "ok none")]
#[case::empty_type(
    200,
    r#"{"contentType":""}"#,
    AUTHORIZED,
    MintScript::Mints,
    "missing_content_type"
)]
#[case::numeric_type(
    200,
    r#"{"contentType":7}"#,
    AUTHORIZED,
    MintScript::Mints,
    "missing_content_type"
)]
#[case::not_json_authorized(200, "not json", AUTHORIZED, MintScript::Mints, "decode")]
#[case::not_json_anonymous(200, "not json", "{}", MintScript::Mints, "ok none")]
#[case::not_an_object_authorized(200, "[1]", AUTHORIZED, MintScript::Mints, "not_an_object")]
#[case::not_an_object_anonymous(200, "[1]", "{}", MintScript::Mints, "ok none")]
#[case::bad_request_authorized(400, r#"{"error":{}}"#, AUTHORIZED, MintScript::Mints, "status 400")]
#[case::forbidden_authorized(403, r#"{"error":{}}"#, AUTHORIZED, MintScript::Mints, "status 403")]
#[case::forbidden_anonymous(403, r#"{"error":{}}"#, "{}", MintScript::Mints, "ok none")]
#[case::server_error(500, "", AUTHORIZED, MintScript::Mints, "status 500")]
#[case::transport_authorized(0, "", AUTHORIZED, MintScript::Mints, "transport")]
#[case::transport_anonymous(0, "", "{}", MintScript::Mints, "ok none")]
#[case::mint_failure(200, PNG_REPLY, AUTHORIZED, MintScript::Fails, "mint")]
#[case::bad_token(200, PNG_REPLY, AUTHORIZED, MintScript::BadToken, "token")]
#[case::numeric_project(
    200,
    PNG_REPLY,
    r#"{"vertex_project":5}"#,
    MintScript::Mints,
    "project"
)]
#[case::numeric_credentials(
    200,
    PNG_REPLY,
    r#"{"vertex_credentials":5}"#,
    MintScript::Mints,
    "credentials"
)]
#[case::undecodable_credentials(
    200,
    PNG_REPLY,
    UNDECODABLE_CREDENTIALS,
    MintScript::Mints,
    "decode_credentials"
)]
fn gcs_metadata_responses(
    #[case] status: u16,
    #[case] body: &str,
    #[case] params: &str,
    #[case] mint: MintScript,
    #[case] expected: &str,
) {
    // a zero status is the scripted transport refusal, which no real reply carries
    let gcs = if status == 0 {
        ScriptedGcs::failing()
    } else {
        ScriptedGcs::answering(status, body)
    };
    let recorder = mint.recorder();
    let deps = GcsDeps {
        transport: &gcs,
        tokens: &recorder,
    };
    let values = parse_object(params);
    let auth = auth_values(&values);
    let reply = block_on(object_content_type(OBJECT, auth, &deps));
    assert_eq!(describe(&reply), expected);
}

#[test]
fn gcs_metadata_request() {
    let gcs = ScriptedGcs::answering(200, PNG_REPLY);
    let mint = MintScript::Mints.recorder();
    let deps = GcsDeps {
        transport: &gcs,
        tokens: &mint,
    };
    let values = parse_object(AUTHORIZED);
    let auth = auth_values(&values);
    let reply = block_on(object_content_type(OBJECT, auth, &deps));
    assert_eq!(describe(&reply), "ok image/png");
    let requests = gcs.take_requests();
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.method, Method::GET);
    assert_eq!(request.url.as_str(), OBJECT_METADATA);
    assert!(request.query.is_empty());
    assert_eq!(request.deadline, METADATA_DEADLINE);
    assert!(matches!(request.body, RequestBody::Empty));
    assert_eq!(header(request, AUTHORIZATION), Some("Bearer token-abc"));
    assert_eq!(header(request, CONTENT_TYPE), None);
    assert_eq!(mint.projects(), vec![Some("p".to_owned())]);
}

#[test]
fn gcs_metadata_anonymous_request() {
    let gcs = ScriptedGcs::answering(200, PNG_REPLY);
    let mint = MintScript::Mints.recorder();
    let deps = GcsDeps {
        transport: &gcs,
        tokens: &mint,
    };
    let params = Object::new();
    let auth = auth_values(&params);
    let reply = block_on(object_content_type(OBJECT, auth, &deps));
    assert_eq!(describe(&reply), "ok image/png");
    let requests = gcs.take_requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(header(&requests[0], AUTHORIZATION), None);
    assert!(mint.projects().is_empty());
}

#[test]
fn replays_the_gcs_metadata_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/gcs_metadata");
    fs::read_dir(corpus)
        .expect("corpus folder")
        .try_for_each(|entry| {
            let path = entry?.path();
            let bytes = fs::read(&path)?;
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .expect("utf-8 file name");
            let decoded = read_content_type(&bytes, true);
            if name.starts_with("valid") {
                decoded.expect("this seed is a metadata reply");
            } else {
                decoded.expect_err("this seed is not a metadata reply");
            }
            Ok::<(), io::Error>(())
        })
        .expect("readable corpus");
}
