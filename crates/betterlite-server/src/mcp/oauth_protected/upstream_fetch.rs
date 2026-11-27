#[cfg(test)]
mod tests;

use betterlite_db::mcp_stored_contracts::JsonValue;
use betterlite_mcp::oauth_metadata_cache::{
    CacheFill, CacheKey, NEGATIVE_TTL_SECONDS, TTL_SECONDS,
};
use betterlite_mcp::server_record::MCPServer;
use betterlite_models::decisions::Json;
use thiserror::Error;

use super::{GetFailure, JsonObject, ProtectedRoot, UpstreamGet, UpstreamReply};

const METADATA_ACCEPT: &str = "application/json";
const METADATA_STATUS_OK: u16 = 200;
const METADATA_WELL_KNOWN_PATH: &str = ".well-known/oauth-protected-resource";

#[derive(Debug, Error)]
pub(crate) enum FetchError {
    #[error("{message}")]
    Transport { message: String },
}

#[derive(Debug)]
pub enum BodyOutcome {
    Object(JsonObject),
    NotObject,
    NotJson(serde_json::Error),
}

#[must_use]
pub fn parse_metadata_body(bytes: &[u8]) -> BodyOutcome {
    match serde_json::from_slice::<Json>(bytes) {
        Ok(Json::Object(document)) => BodyOutcome::Object(document),
        Ok(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            BodyOutcome::NotObject
        }
        Err(error) => BodyOutcome::NotJson(error),
    }
}

struct UpstreamTarget {
    host_base: String,
    path: String,
}

fn upstream_target(url: &str) -> Option<UpstreamTarget> {
    let (scheme, rest) = url.split_once("://")?;
    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let netloc = rest.get(..authority_end)?;
    if scheme.is_empty() || netloc.is_empty() {
        return None;
    }
    let after_authority = rest.get(authority_end..).unwrap_or_default();
    let path = after_authority.split(['?', '#']).next().unwrap_or_default();
    let lowered = scheme.to_ascii_lowercase();
    Some(UpstreamTarget {
        host_base: format!("{lowered}://{netloc}"),
        path: path.to_owned(),
    })
}

fn metadata_candidates(target: &UpstreamTarget) -> Vec<String> {
    let host_base = target.host_base.as_str();
    let host_candidate = format!("{host_base}/{METADATA_WELL_KNOWN_PATH}");
    let mut candidates = vec![host_candidate];
    let trimmed = target.path.trim_end_matches('/');
    if !target.path.is_empty() && target.path != "/" {
        candidates.push(format!("{host_base}/{METADATA_WELL_KNOWN_PATH}{trimmed}"));
    }
    candidates
}

fn payload_from(candidate: &str, reply: &UpstreamReply) -> Option<JsonObject> {
    if reply.status != METADATA_STATUS_OK {
        return None;
    }
    match parse_metadata_body(&reply.body) {
        BodyOutcome::Object(document) => Some(document),
        BodyOutcome::NotObject => None,
        BodyOutcome::NotJson(error) => {
            tracing::warn!(
                "MCP OAuth metadata at {candidate} returned 200 but JSON decode failed ({error}), treating as no metadata"
            );
            None
        }
    }
}

fn cached_document(payload: JsonValue) -> Option<JsonObject> {
    match payload {
        JsonValue::Object(entries) => Some(
            entries
                .into_iter()
                .map(|(field, value)| (field, document_value(value)))
                .collect(),
        ),
        JsonValue::Null
        | JsonValue::Bool(_)
        | JsonValue::Number(_)
        | JsonValue::String(_)
        | JsonValue::Array(_) => None,
    }
}

fn document_value(value: JsonValue) -> Json {
    match value {
        JsonValue::Null => Json::Null,
        JsonValue::Bool(flag) => Json::Bool(flag),
        JsonValue::Number(number) => Json::Number(number),
        JsonValue::String(text) => Json::Text(text),
        JsonValue::Array(items) => Json::List(items.into_iter().map(document_value).collect()),
        JsonValue::Object(entries) => Json::Object(
            entries
                .into_iter()
                .map(|(field, item)| (field, document_value(item)))
                .collect(),
        ),
    }
}

fn stored_value(value: &Json) -> JsonValue {
    match value {
        Json::Null => JsonValue::Null,
        Json::Bool(flag) => JsonValue::Bool(*flag),
        Json::Number(number) => JsonValue::Number(number.clone()),
        Json::Text(text) => JsonValue::String(text.clone()),
        Json::List(items) => JsonValue::Array(items.iter().map(stored_value).collect()),
        Json::Object(entries) => JsonValue::Object(
            entries
                .iter()
                .map(|(field, item)| (field.clone(), stored_value(item)))
                .collect(),
        ),
    }
}

impl<U, R, G> ProtectedRoot<U, R, G>
where
    U: UpstreamGet,
{
    // ast-grep-ignore: no-long-names
    pub(crate) async fn fetch_upstream_oauth_protected_resource(
        &self,
        server: &MCPServer,
    ) -> Result<Option<JsonObject>, FetchError> {
        let Some(url) = server.fields.url.as_deref() else {
            return Ok(None);
        };
        if url.is_empty() {
            return Ok(None);
        }
        let Some(target) = upstream_target(url) else {
            return Ok(None);
        };
        let key = CacheKey {
            server_id: server.fields.server_id.clone(),
            resource_url: url.to_owned(),
        };
        let now = (self.clock)();
        self.cache.prune(now);
        let first = self.cache.lookup(&key, now);
        if first.is_some() {
            return Ok(first.and_then(cached_document));
        }
        let _slot = self.cache.enter_fetch(&key).await;
        let second = self.cache.lookup(&key, (self.clock)());
        if second.is_some() {
            return Ok(second.and_then(cached_document));
        }
        let generation = self.cache.generation(&server.fields.server_id);
        self.fetch_candidates(&key, generation, &target).await
    }

    async fn fetch_candidates(
        &self,
        key: &CacheKey,
        generation: u64,
        target: &UpstreamTarget,
    ) -> Result<Option<JsonObject>, FetchError> {
        let candidates = metadata_candidates(target);
        let mut transport_failures: Vec<String> = Vec::new();
        for candidate in &candidates {
            match self.fetch_candidate(candidate, key, generation).await {
                Ok(Some(document)) => return Ok(Some(document)),
                Ok(None) => {}
                Err(GetFailure::Transport(message)) => transport_failures.push(message),
                Err(GetFailure::Other(kind)) => {
                    tracing::warn!(
                        "MCP OAuth metadata fetch for {candidate} raised non-transport {kind}, treating as no metadata for this candidate"
                    );
                }
            }
        }
        if transport_failures.len() == candidates.len() {
            return Err(FetchError::Transport {
                message: transport_failures.last().cloned().unwrap_or_default(),
            });
        }
        let _stored = self.cache.store(
            key,
            generation,
            CacheFill {
                payload: None,
                ttl_seconds: NEGATIVE_TTL_SECONDS,
            },
            (self.clock)(),
        );
        Ok(None)
    }

    async fn fetch_candidate(
        &self,
        candidate: &str,
        key: &CacheKey,
        generation: u64,
    ) -> Result<Option<JsonObject>, GetFailure> {
        let reply = self.upstream.get(candidate, METADATA_ACCEPT).await?;
        let Some(document) = payload_from(candidate, &reply) else {
            return Ok(None);
        };
        let _stored = self.cache.store(
            key,
            generation,
            CacheFill {
                payload: Some(JsonValue::Object(
                    document
                        .iter()
                        .map(|(field, value)| (field.clone(), stored_value(value)))
                        .collect(),
                )),
                ttl_seconds: TTL_SECONDS,
            },
            (self.clock)(),
        );
        Ok(Some(document))
    }
}
