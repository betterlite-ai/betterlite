use std::ffi::OsString;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::sections::huggingface_hub::{OfflineSetting, Settings as HubSettings};
use reqwest::header::{HeaderMap, HeaderValue};
use reqwest::{Method, Url};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use tokio::sync::Mutex;

use crate::hub_oauth::clock::HubClock;
use crate::hub_oauth::error::RefreshError;
use crate::hub_oauth::file_lock;
use crate::hub_oauth::secret_file::write_secret;
use crate::hub_oauth::stored_tokens::{field, parse::Section, read_full, save_token};
use crate::hub_token::{HubTokenError, HubTokenFileReader, read_token_file};
use crate::proxy::ProxiedClient;
use crate::proxy::outbound::{OutboundRequest, RequestBody};

const REFRESH_MARGIN: i64 = 24 * 3600;
const RECHECK_INTERVAL: i64 = 300;
pub const LOCK_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Deserialize)]
struct TokenAnswer {
    access_token: Option<String>,
    refresh_token: Option<String>,
    expires_in: Option<i64>,
    error: Option<String>,
    error_description: Option<String>,
}

struct Cached {
    file_token: SecretString,
    resolved_token: SecretString,
    recheck_at: i64,
}

// a lookup either caches its decision or serves a token that must not replace the cache
enum Decision {
    Cache(Cached),
    Serve(SecretString),
}

#[derive(Default)]
pub(crate) struct State {
    cache: Option<Cached>,
    warned: bool,
}

pub struct OauthRefresh {
    pub(crate) lock_timeout: Duration,
    pub(crate) hub: Arc<HubSettings>,
    pub(crate) client_id: String,
    pub(crate) client: Arc<ProxiedClient>,
    pub(crate) clock: Arc<dyn HubClock>,
    pub(crate) file_reader: Arc<dyn HubTokenFileReader>,
    pub(crate) state: Mutex<State>,
}

impl OauthRefresh {
    // huggingface_hub's _refresh_oauth_token_if_needed: a failed refresh is warned about and keeps the file token
    pub async fn refresh_if_needed(
        &self,
        token: SecretString,
    ) -> Result<SecretString, HubTokenError> {
        let mut state = self.state.lock().await;
        let now = self.clock.unix_seconds();
        if let Some(cached) = &state.cache
            && cached.file_token.expose_secret() == token.expose_secret()
            && now < cached.recheck_at
        {
            return Ok(cached.resolved_token.clone());
        }
        match self.check(&mut state.warned, token, now).await? {
            Decision::Cache(checked) => {
                let resolved = checked.resolved_token.clone();
                state.cache = Some(checked);
                drop(state);
                Ok(resolved)
            }
            Decision::Serve(replaced) => {
                // huggingface_hub returns this token without touching its cache
                drop(state);
                Ok(replaced)
            }
        }
    }

    async fn check(
        &self,
        warned: &mut bool,
        token: SecretString,
        now: i64,
    ) -> Result<Decision, HubTokenError> {
        let sections = read_full(&self.hub.stored_tokens_path).await?;
        let Some((name, refresh_token, stored_expiry)) = refreshable(&sections, &token) else {
            return self.replaced_or(token, now).await;
        };
        if stored_expiry.saturating_sub(REFRESH_MARGIN) > now {
            return Ok(Decision::Cache(Cached {
                file_token: token.clone(),
                resolved_token: token,
                recheck_at: stored_expiry.saturating_sub(REFRESH_MARGIN),
            }));
        }
        match self
            .refresh_locked(&name, &token, &refresh_token, now)
            .await
        {
            Ok((new_token, new_expires_at)) => Ok(Decision::Cache(Cached {
                file_token: new_token.clone(),
                resolved_token: new_token,
                // the floor keeps a token that lives less than the margin from being refreshed on every call
                recheck_at: now.saturating_add(RECHECK_INTERVAL).max(
                    new_expires_at.map_or(0, |expires| expires.saturating_sub(REFRESH_MARGIN)),
                ),
            })),
            Err(error) => Ok(Decision::Cache(Cached {
                file_token: token.clone(),
                resolved_token: token,
                recheck_at: warn_about(&error, warned, now),
            })),
        }
    }

    // a concurrent refresh may have replaced the token, in which case the new one is served and not kept
    async fn replaced_or(&self, token: SecretString, now: i64) -> Result<Decision, HubTokenError> {
        let on_disk = read_token_file(&*self.file_reader, &self.hub.token_path).await?;
        if let Some(current) =
            on_disk.filter(|current| current.expose_secret() != token.expose_secret())
        {
            return Ok(Decision::Serve(current));
        }
        Ok(Decision::Cache(Cached {
            file_token: token.clone(),
            resolved_token: token,
            recheck_at: now.saturating_add(RECHECK_INTERVAL),
        }))
    }

    async fn refresh_locked(
        &self,
        name: &str,
        token: &SecretString,
        refresh_token: &SecretString,
        now: i64,
    ) -> Result<(SecretString, Option<i64>), RefreshError> {
        let mut lock_path = OsString::from(&self.hub.stored_tokens_path);
        lock_path.push(".lock");
        let _lock = file_lock::acquire(lock_path.as_ref(), self.lock_timeout).await?;
        let sections = read_full(&self.hub.stored_tokens_path).await?;
        let current = sections.iter().find(|section| section.name == name);
        let current_token = current.and_then(|section| field(section, "hf_token"));
        if current_token.is_none_or(|stored| stored.expose_secret() != token.expose_secret()) {
            // another process refreshed this token already
            let adopted = current_token
                .filter(|stored| !stored.expose_secret().is_empty())
                .unwrap_or(token)
                .clone();
            let adopted_expiry = current
                .and_then(|section| field(section, "expires_at"))
                .and_then(|stored| stored.expose_secret().parse().ok());
            return Ok((adopted, adopted_expiry));
        }
        let answer = self.request_refresh(refresh_token).await?;
        let new_token = SecretString::from(answer.access_token.unwrap_or_default());
        let new_expires_at = answer.expires_in.map(|seconds| now.saturating_add(seconds));
        let rotated = answer
            .refresh_token
            .filter(|rotated| !rotated.is_empty())
            .map_or_else(|| refresh_token.clone(), SecretString::from);
        save_token(
            &self.hub.stored_tokens_path,
            name,
            &new_token,
            &rotated,
            new_expires_at,
        )
        .await?;
        self.replace_token_file(token, &new_token).await?;
        tracing::info!("Access token `{name}` has been refreshed.");
        Ok((new_token, new_expires_at))
    }

    // the token file is rewritten only while it still holds the token that was refreshed
    async fn replace_token_file(
        &self,
        old: &SecretString,
        new: &SecretString,
    ) -> Result<(), RefreshError> {
        let on_disk = read_token_file(&*self.file_reader, &self.hub.token_path).await?;
        if on_disk.is_some_and(|stored| stored.expose_secret() == old.expose_secret()) {
            write_secret(&self.hub.token_path, new.expose_secret())
                .await
                .map_err(RefreshError::Save)?;
        }
        Ok(())
    }

    async fn request_refresh(
        &self,
        refresh_token: &SecretString,
    ) -> Result<TokenAnswer, RefreshError> {
        let url = format!("{}/oauth/token", self.hub.endpoint);
        // huggingface_hub's request hook refuses every Hub call in offline mode, so nothing is sent
        if matches!(self.hub.offline, OfflineSetting::Offline) {
            return Err(RefreshError::Offline { url });
        }
        let form = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("grant_type", "refresh_token")
            .append_pair("refresh_token", refresh_token.expose_secret())
            .append_pair("client_id", &self.client_id)
            .finish();
        let request = OutboundRequest {
            method: Method::POST,
            url: Url::parse(&url).map_err(|source| RefreshError::Url {
                url: url.clone(),
                source,
            })?,
            query: Vec::new(),
            headers: HeaderMap::new(),
            body: RequestBody::Bytes {
                content_type: HeaderValue::from_static("application/x-www-form-urlencoded"),
                data: form.into_bytes(),
            },
            deadline: Duration::from_secs(self.hub.download_timeout),
        };
        let response = self
            .client
            .send_buffered(request)
            .await
            .map_err(RefreshError::Request)?;
        read_answer(url, response.status, &response.body)
    }
}

fn refreshable(sections: &[Section], token: &SecretString) -> Option<(String, SecretString, i64)> {
    let stored = sections.iter().find(|section| {
        field(section, "hf_token")
            .is_some_and(|stored| stored.expose_secret() == token.expose_secret())
    })?;
    Some((
        stored.name.clone(),
        field(stored, "refresh_token")?.clone(),
        field(stored, "expires_at")?.expose_secret().parse().ok()?,
    ))
}

fn warn_about(error: &RefreshError, warned: &mut bool, now: i64) -> i64 {
    if matches!(error, RefreshError::Rejected { error: Some(code), .. } if code == "invalid_grant")
    {
        tracing::warn!(
            "Your Hugging Face access token has expired and could not be refreshed (session expired or revoked). Run `hf auth login` to re-authenticate. ({error})"
        );
        return i64::MAX;
    }
    if !*warned {
        tracing::warn!(
            "Could not refresh your Hugging Face access token: {error}. Will retry later."
        );
        *warned = true;
    }
    now.saturating_add(RECHECK_INTERVAL)
}

// success is a body with an access_token whatever the status, the way requests-based huggingface_hub reads it
fn read_answer(url: String, status: u16, body: &[u8]) -> Result<TokenAnswer, RefreshError> {
    let answer: TokenAnswer =
        serde_json::from_slice(body).map_err(|source| RefreshError::Unreadable {
            url,
            status,
            source,
        })?;
    if answer.access_token.is_none() {
        return Err(RefreshError::Rejected {
            status,
            error: answer.error,
            description: answer.error_description.unwrap_or_default(),
        });
    }
    Ok(answer)
}
