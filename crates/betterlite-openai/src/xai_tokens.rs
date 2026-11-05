use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use betterlite_config::sections::xai_oauth_settings::Settings;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use http::header::ACCEPT;
use http::{HeaderMap, HeaderValue, Method};
use rust_decimal::Decimal;
use secrecy::{ExposeSecret as _, SecretString};
use tokio::sync::Mutex;
use url::{Url, form_urlencoded};

use self::codec::{AuthFile, DiscoveryDocument, TokenPayload, decode_xai_oauth};
use self::error::XaiOauthError;
use self::files::FileAccess;
use self::rules::{
    CLIENT_ID, DISCOVERY_URL, OauthRecord, build_auth_record, is_expired, validate_xai_endpoint,
};
use self::transport::Transport;

pub mod codec;
pub mod error;
pub mod files;
pub mod rules;
mod tests;
pub mod transport;

pub trait Clock: Send + Sync {
    fn now(&self) -> Decimal;
}

pub struct XaiOauth<F, C, T> {
    pub settings: Settings,
    pub files: F,
    pub clock: C,
    pub transport: T,
    pub deadline: Duration,
    pub auth_file: PathBuf,
    pub refresh_lock: Mutex<()>,
}

impl<F: FileAccess, C: Clock, T: Transport> XaiOauth<F, C, T> {
    pub fn new(settings: Settings, files: F, clock: C, transport: T, deadline: Duration) -> Self {
        let auth_file = Path::new(&settings.token_dir).join(&settings.auth_file);
        Self {
            settings,
            files,
            clock,
            transport,
            deadline,
            auth_file,
            refresh_lock: Mutex::new(()),
        }
    }

    pub async fn get_access_token(&self) -> Result<SecretString, XaiOauthError> {
        let auth = self.read_auth_file().ok_or(XaiOauthError::LoginRequired)?;
        if let Some(token) = self.unexpired_token(&auth) {
            return Ok(token);
        }
        if auth
            .refresh_token
            .as_ref()
            .is_none_or(|token| token.expose_secret().is_empty())
        {
            return Err(XaiOauthError::RefreshTokenMissing);
        }
        let _refreshing = self.refresh_lock.lock().await;
        let latest = self.read_auth_file().unwrap_or(auth);
        if let Some(token) = self.unexpired_token(&latest) {
            return Ok(token);
        }
        Ok(self.refresh_tokens(&latest).await?.access_token)
    }

    pub fn read_auth_file(&self) -> Option<AuthFile> {
        let bytes = self.files.read(&self.auth_file).ok()?;
        decode_xai_oauth(&bytes)
            .map_err(|error| tracing::debug!(%error, "Ignoring an unreadable xAI OAuth auth file"))
            .ok()
    }

    pub fn write_auth_file(&self, record: &OauthRecord) -> Result<(), XaiOauthError> {
        let contents = serde_json::to_vec(&record.stored()).map_err(io::Error::other);
        let directory = Path::new(&self.settings.token_dir);
        self.files
            .ensure_dir(directory)
            .and_then(|()| self.files.replace(directory, &self.auth_file, &contents?))
            .map_err(XaiOauthError::Write)
    }

    fn unexpired_token(&self, auth: &AuthFile) -> Option<SecretString> {
        auth.access_token
            .as_ref()
            .filter(|token| !token.expose_secret().is_empty())
            .filter(|_| !is_expired(self.clock.now(), auth.expires_at.as_ref()))
            .cloned()
    }

    pub async fn discover(&self) -> Result<(String, String), XaiOauthError> {
        let location = validate_xai_endpoint(DISCOVERY_URL)?;
        let body = self
            .send("discovery", Method::GET, location, RequestBody::Empty)
            .await?;
        let document: DiscoveryDocument =
            decode_xai_oauth(&body).map_err(|source| XaiOauthError::Response {
                stage: "discovery",
                source,
            })?;
        let (Some(authorization), Some(token)) = (
            document
                .authorization_endpoint
                .filter(|url| !url.is_empty()),
            document.token_endpoint.filter(|url| !url.is_empty()),
        ) else {
            return Err(XaiOauthError::MissingEndpoints);
        };
        validate_xai_endpoint(&authorization)?;
        validate_xai_endpoint(&token)?;
        Ok((authorization, token))
    }

    pub async fn exchange_token(
        &self,
        token_endpoint: Url,
        fields: &[(&str, &str)],
    ) -> Result<TokenPayload, XaiOauthError> {
        let form = form_urlencoded::Serializer::new(String::new())
            .extend_pairs(fields)
            .finish();
        let body = RequestBody::Bytes {
            content_type: HeaderValue::from_static("application/x-www-form-urlencoded"),
            data: form.into_bytes(),
        };
        let reply = self
            .send("token", Method::POST, token_endpoint, body)
            .await?;
        decode_xai_oauth(&reply).map_err(|source| XaiOauthError::Response {
            stage: "token",
            source,
        })
    }

    pub async fn refresh_tokens(&self, auth: &AuthFile) -> Result<OauthRecord, XaiOauthError> {
        let endpoint = match auth.token_endpoint.as_deref().filter(|url| !url.is_empty()) {
            Some(stored) => stored.to_owned(),
            None => self.discover().await?.1,
        };
        let url = validate_xai_endpoint(&endpoint)?;
        let refresh_token = auth
            .refresh_token
            .as_ref()
            .filter(|token| !token.expose_secret().is_empty())
            .ok_or(XaiOauthError::RefreshTokenMissing)?;
        let fields = [
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token.expose_secret()),
            ("client_id", CLIENT_ID),
        ];
        let payload = self.exchange_token(url, &fields).await?;
        let record = build_auth_record(self.clock.now(), payload, &endpoint, Some(refresh_token))?;
        self.write_auth_file(&record)?;
        Ok(record)
    }

    async fn send(
        &self,
        stage: &'static str,
        method: Method,
        url: Url,
        body: RequestBody,
    ) -> Result<Vec<u8>, XaiOauthError> {
        let request = OutboundRequest {
            method,
            url,
            query: Vec::new(),
            headers: HeaderMap::from_iter([(ACCEPT, HeaderValue::from_static("application/json"))]),
            body,
            deadline: self.deadline,
        };
        let response = (self.transport.send(request).await)
            .map_err(|source| XaiOauthError::Transport { stage, source })?;
        if (200..300).contains(&response.status) {
            Ok(response.body)
        } else {
            Err(XaiOauthError::Status {
                stage,
                status: response.status,
            })
        }
    }
}
