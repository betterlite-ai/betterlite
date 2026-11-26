use betterlite_models::decisions::Json;

use super::super::decode_discovery_document;
use super::super::errors::KeyError;
use super::super::http::{FetchError, HttpReply};
use super::{JWKS_FETCH_ATTEMPTS, JWKS_FETCH_RETRY_BACKOFF, JwksResolver, Refresh};

impl JwksResolver {
    pub(super) async fn refresh_now(&self, refresh: Refresh<'_>) -> Result<Json, KeyError> {
        match refresh {
            Refresh::Discovery(url) => self.fetch_jwks_uri_from_discovery(url).await,
            Refresh::Keys(url) => self.fetch_jwks_keys(url).await,
        }
    }

    // _get_with_transient_retries: three attempts, waiting 0.25 s then 0.5 s, and only transport errors retry.
    async fn get_with_transient_retries(&self, url: &str) -> Result<HttpReply, KeyError> {
        for attempt in 1..JWKS_FETCH_ATTEMPTS {
            match self.http.get(url).await {
                Ok(reply) => return Ok(reply),
                Err(FetchError::Transport(source)) => {
                    tracing::warn!(
                        target: "LiteLLM Proxy",
                        "JWT Auth: transport error fetching {url} (attempt {attempt}/{JWKS_FETCH_ATTEMPTS}), \
                         retrying: {source}"
                    );
                    self.clock.sleep(JWKS_FETCH_RETRY_BACKOFF * attempt).await;
                }
                Err(source @ FetchError::Failed(_)) => {
                    return Err(KeyError::Fetch {
                        url: url.to_owned(),
                        source,
                    });
                }
            }
        }
        match self.http.get(url).await {
            Ok(reply) => Ok(reply),
            Err(FetchError::Transport(source)) => Err(KeyError::Unreachable {
                message: format!("{source} fetching {url} after {JWKS_FETCH_ATTEMPTS} attempts"),
            }),
            Err(source @ FetchError::Failed(_)) => Err(KeyError::Fetch {
                url: url.to_owned(),
                source,
            }),
        }
    }

    // _fetch_jwks_uri_from_discovery: exactly 200, valid JSON and a truthy jwks_uri, each with its own message.
    async fn fetch_jwks_uri_from_discovery(&self, url: &str) -> Result<Json, KeyError> {
        tracing::debug!(
            target: "LiteLLM Proxy",
            "JWT Auth: Fetching OIDC discovery document from {url}"
        );
        let reply = self.get_with_transient_retries(url).await?;
        if reply.status != 200 {
            return Err(KeyError::DiscoveryStatus {
                url: url.to_owned(),
                status: reply.status,
                body: body_text(&reply.body),
            });
        }
        let document =
            decode_discovery_document(&reply.body).map_err(|source| KeyError::DiscoveryParse {
                url: url.to_owned(),
                source,
            })?;
        let Json::Object(fields) = &document else {
            return Err(KeyError::DiscoveryMissingUri {
                url: url.to_owned(),
            });
        };
        let Some(Json::Text(jwks_uri)) = fields.get("jwks_uri") else {
            return Err(KeyError::DiscoveryMissingUri {
                url: url.to_owned(),
            });
        };
        if jwks_uri.is_empty() {
            return Err(KeyError::DiscoveryMissingUri {
                url: url.to_owned(),
            });
        }
        tracing::debug!(
            target: "LiteLLM Proxy",
            "JWT Auth: Resolved OIDC discovery {url} -> jwks_uri={jwks_uri}"
        );
        Ok(Json::Text(jwks_uri.clone()))
    }

    // _fetch_jwks_keys: exactly 200; a parse failure logs the body and raises its own message; the keys member
    // is used when present, else the response itself, and key material is never reduced to kid.
    async fn fetch_jwks_keys(&self, url: &str) -> Result<Json, KeyError> {
        let reply = self.get_with_transient_retries(url).await?;
        if reply.status != 200 {
            return Err(KeyError::JwksStatus {
                url: url.to_owned(),
                status: reply.status,
                body: body_text(&reply.body),
            });
        }
        let document = match decode_discovery_document(&reply.body) {
            Ok(document) => document,
            Err(source) => {
                tracing::error!(
                    target: "LiteLLM Proxy",
                    "Error parsing response: {source}. Original Response: {body}",
                    body = body_text(&reply.body)
                );
                return Err(KeyError::JwksParse { source });
            }
        };
        Ok(match document {
            Json::Object(mut fields) => fields.shift_remove("keys").unwrap_or(Json::Object(fields)),
            other => other,
        })
    }
}

fn body_text(bytes: &[u8]) -> String {
    let mut text = String::new();
    for chunk in bytes.utf8_chunks() {
        text.push_str(chunk.valid());
        if !chunk.invalid().is_empty() {
            text.push('\u{FFFD}');
        }
    }
    text
}
