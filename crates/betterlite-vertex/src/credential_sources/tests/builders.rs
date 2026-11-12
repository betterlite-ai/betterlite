#![cfg(test)]

use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::Arc;

use betterlite_bedrock::aws_settings::sources::Credentials;
use betterlite_bedrock::credentials::error::CredentialError as BedrockCredentialError;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use secrecy::{ExposeSecret as _, SecretString};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;

use super::fixtures::FixedClock;
use crate::aws_federation::AwsCredentialsResolver;
use crate::aws_federation::FederationInput;
use crate::credential_sources::error::CredentialSourceError;
use crate::credential_sources::sdk::{CredentialBuilders, GoogleAuth, HttpPolicy};

// a throwaway test key, never a real service account
const SERVICE_ACCOUNT_KEY: &str = "-----BEGIN PRIVATE KEY-----
MIIEvgIBADANBgkqhkiG9w0BAQEFAASCBKgwggSkAgEAAoIBAQDEZk18hkLMU1zI
FZmSh6nWE0SBotTd4ECQHQiLfg8FaUqYOV51zGKAElfCIhzWNhJcvLfZSwnreIlD
mwwBzgxoOT2Sa8U6TlLPmrd1ty2yTisef8iJY3xGWArPEcQst8g4KvAS7G07ct8/
cr+4yqsPPo2STPeR/gb3nXuXcXqb9W6W9jZOuX3CaS3kXXwIjQ+XUptS8wJo0zn/
gYBoRTV7ajiQQl8Rk2izqDI7mmRDsu7Fqam00fG5FwVBtpzKwysDzEbY1eDUvxag
+t7ZopQ/i9tAZtK+lY6+w8lso0NwYp3AfNxACnL7X1QfpBZmzVaTQ10NlRTaihmy
7aPWKABrAgMBAAECggEAA5Nd53L6+1pXLu4Jvju0ZrPcxEQb07BLV0mtXVDIYuUA
WSt44fR8efAbjerDaKOntkLZ/AhG4B/GgVHCKF7r0JKeNttkcK56s9HmyNYIFx4f
iy4bZLjyyxsQ6KUzqPbC8UGC3m1Ugx0RHcgainmSH18YV74JUddJ9tnJ+r9eUpbf
UEpMyNrZ1LY5AsXEP5NL9TB4wEH7FaZVR23uZXmNAFZ/FJfHwaHyKg+HGOMNFbin
lBGWJIBnYeZ1tB+dpq1AEFbeSG8PgSJsNJiWBMLUci9E7SAO1SSs437IXzkKFBIs
oAkZdkDKLM26iVJvsEmDEo1J3UH4zkcemj3dv91HOQKBgQDrtQcomPJTWzB8bXcP
z0g+DKei0oxrYDQk+E1OF2QrDbrZgEDCoiE8Wav4/8pjMd7fix6a3aM4GvteQR1/
FlyTYB4LrVcoUWMYwMSWT1MKIbiaq6v0C9/LSDp3OTnEFJXCQ6vtGfRYGpCBCY64
w27eEfHu38HaAZluq/CXThclIwKBgQDVTvBunrhSTAhxK6OVmGDrCnx/HxYFyaGi
5mdDJJWzhZKaS9x9V/HOO160zMjASX1i7cTDg0gRRQetK7oG71E6boVwfljLIzfx
7aF2/Q8FcmKfb92Z49dn9FYSJ6qWCsBfACWvGZaRQbxoOhlMCE9WxUEtjV9/4W5Z
/bKi0EsgGQKBgQCnCcdk8d7pS7p0s26o+ZCMyJMURHnWrnrFqGHAcbs+95/u+4Dp
bdR33GSPNVbMLU9wxDqQLXm6DQUOqr6ojcRVfMxOIi2eGO9NX27xxo1mpLcahYZ6
kN+sRCeKzjctyhjHbwWsJbSPq2O0zn0iWv16yuTVECVkxbViRN9fmqu0WQKBgQDP
3p+bjX1eZc4MS1SdIePA6XUthKO3fCjSUOnsmqhwTAjz6F20oJF6Qo0mcDJqgLN6
Ru9rAHKosbarYWNmAq4L3UyYtefd/jI0xziEdlO/IRtO+HrhzxFJsuh1xMYOdefG
LKSMoMkEQSVUBNpGKAr5Kj08IPsh6hPaCsucspOjcQKBgDWnwMJATRZUQGPQxS+H
1rNwPwGHi9xWNl6130o6n8wtPJGDQhUmJ4geimtnWCxqSq/sbWnT19LUQD07YrF0
Mn3qTYagYmvgrAjrzPEaKHOTDqtk+Vv2edH+qcmQUAqPhvVFseamX9d24KHiv15z
LThjB2npemj4BJAh5S9lt+u+
-----END PRIVATE KEY-----\n";

const CLOUD_PLATFORM: &str = "https://www.googleapis.com/auth/cloud-platform";
const STS_ANSWER: &str = r#"{"access_token":"ya29.sts-token","expires_in":3599}"#;

fn scopes() -> Vec<String> {
    vec![CLOUD_PLATFORM.to_owned()]
}

fn google_auth() -> GoogleAuth {
    let policy = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap());
    GoogleAuth {
        client: ProxiedClient::outbound(&policy).unwrap(),
        policy: HttpPolicy::default(),
    }
}

struct StaticAws;

impl AwsCredentialsResolver for StaticAws {
    fn credentials(
        &self,
        _settings: &AwsSettings,
    ) -> impl Future<Output = Result<Credentials, BedrockCredentialError>> + Send {
        ready(Ok(Credentials {
            access_key_id: SecretString::from("AKIAEXAMPLE".to_owned()),
            secret_access_key: SecretString::from("aws-secret".to_owned()),
            session_token: None,
        }))
    }
}

struct Upstream {
    host: String,
    _tasks: JoinSet<()>,
}

async fn serve(mut socket: TcpStream, status: u16, body: &'static str) {
    let mut received = Vec::new();
    while !received.ends_with(b"\r\n\r\n") {
        received.push(socket.read_u8().await.unwrap());
    }
    let head = String::from_utf8(received).unwrap();
    let content_length = head
        .lines()
        .filter_map(|line| line.split_once(": "))
        .find(|(name, _value)| name.eq_ignore_ascii_case("content-length"))
        .map_or(0, |(_name, value)| value.trim().parse().unwrap());
    let mut content = vec![0; content_length];
    socket.read_exact(&mut content).await.unwrap();
    let reply = format!(
        "HTTP/1.1 {status} Reply\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    socket.write_all(reply.as_bytes()).await.unwrap();
}

async fn upstream(status: u16, body: &'static str) -> Upstream {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let host = listener.local_addr().unwrap().to_string();
    let mut tasks = JoinSet::new();
    tasks.spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        serve(socket, status, body).await;
    });
    Upstream {
        host,
        _tasks: tasks,
    }
}

fn aws_document(host: &str, region: Option<&str>) -> String {
    let region = region.map_or_else(String::new, |region| {
        format!(r#","aws_region_name":"{region}""#)
    });
    format!(
        r#"{{"type":"external_account","audience":"audience","subject_token_type":"urn:ietf:params:oauth:token-type:aws4_request","token_url":"http://{host}/v1/token","credential_source":{{"environment_id":"aws1"}},"aws_access_key_id":"AKIAEXAMPLE","aws_secret_access_key":"aws-secret"{region}}}"#
    )
}

#[tokio::test]
async fn builds_a_service_account_credential() {
    let document = serde_json::json!({
        "type": "service_account",
        "project_id": "project",
        "private_key_id": "key-id",
        "private_key": SERVICE_ACCOUNT_KEY,
        "client_email": "sa@example.iam.gserviceaccount.com",
    })
    .to_string();
    let source = google_auth().service_account(&document, &scopes()).unwrap();
    let token = source.access_token().await.unwrap();
    assert!(token.expose_secret().starts_with("ey"));
}

#[tokio::test]
async fn builds_an_authorized_user_credential() {
    let document = serde_json::json!({
        "type": "authorized_user",
        "client_id": "client",
        "client_secret": "client-secret",
        "refresh_token": "refresh-token",
    })
    .to_string();
    assert!(google_auth().authorized_user(&document, &scopes()).is_ok());
}

#[tokio::test]
async fn builds_an_identity_pool_credential() {
    let document = serde_json::json!({
        "type": "external_account",
        "audience": "audience",
        "subject_token_type": "urn:ietf:params:oauth:token-type:jwt",
        "credential_source": {"file": "/run/token"},
    })
    .to_string();
    assert!(google_auth().identity_pool(&document, &scopes()).is_ok());
}

#[tokio::test]
async fn builds_an_identity_pool_with_aws_credential() {
    let document = serde_json::json!({
        "type": "external_account",
        "audience": "audience",
        "subject_token_type": "urn:ietf:params:oauth:token-type:aws4_request",
        "credential_source": {"environment_id": "aws1"},
    })
    .to_string();
    assert!(
        google_auth()
            .identity_pool_with_aws(&document, &scopes())
            .is_ok()
    );
}

#[tokio::test]
async fn refuses_the_pluggable_source() {
    let error = google_auth()
        .pluggable("{}", &scopes())
        .err()
        .expect("the pluggable source is refused");
    assert!(matches!(error, CredentialSourceError::ExecutableSource));
}

#[tokio::test]
async fn builds_metadata_credentials() {
    let source = google_auth()
        .metadata(Some("127.0.0.1:1"), &scopes())
        .unwrap();
    assert!(
        source.access_token().await.is_err(),
        "no metadata server listens on the given endpoint"
    );
}

#[tokio::test]
async fn metadata_project_reads_the_project() {
    let server = upstream(200, "metadata-project").await;
    let project = google_auth().metadata_project(Some(&server.host)).await;
    assert_eq!(project.as_deref(), Some("metadata-project"));
}

#[tokio::test]
async fn metadata_project_returns_none_on_a_non_200() {
    let server = upstream(500, "boom").await;
    assert_eq!(
        google_auth().metadata_project(Some(&server.host)).await,
        None
    );
}

#[tokio::test]
async fn metadata_project_returns_none_for_a_malformed_host() {
    assert_eq!(google_auth().metadata_project(Some("bad host")).await, None);
}

#[tokio::test]
async fn aws_federation_refreshes_through_the_resolver() {
    let server = upstream(200, STS_ANSWER).await;
    let input: FederationInput =
        serde_json::from_str(&aws_document(&server.host, Some("us-east-1"))).unwrap();
    let source = google_auth()
        .aws_federation(input, Arc::new(StaticAws), Arc::new(FixedClock))
        .unwrap();
    let token = source.access_token().await.unwrap();
    assert_eq!(token.expose_secret(), "ya29.sts-token");
}

#[tokio::test]
async fn aws_federation_reports_a_missing_region() {
    let server = upstream(200, STS_ANSWER).await;
    let input: FederationInput = serde_json::from_str(&aws_document(&server.host, None)).unwrap();
    let source = google_auth()
        .aws_federation(input, Arc::new(StaticAws), Arc::new(FixedClock))
        .unwrap();
    let error = source.access_token().await.unwrap_err();
    assert!(matches!(error, CredentialSourceError::Federation(_)));
}
