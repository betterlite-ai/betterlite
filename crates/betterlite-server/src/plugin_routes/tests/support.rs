#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{HeaderMap, Request, StatusCode};
use betterlite_auth::auth_identity::records::{
    CredentialRef, NetworkContext, Principal, SourceKey, UserIdentity,
};
use betterlite_auth::auth_identity::roles::{AuthMethod, PrincipalType, Role};
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_http::error::HttpError;
use betterlite_pyhost::json_values::{JsonObject, JsonValue};
use tower::ServiceExt as _;

use super::config::PluginRegistry;
use super::headers::KeyHeaderName;
use super::proxy::{PluginReply, PluginRequest, PluginSendError, PluginState, PluginUpstream};
use crate::auth::auth_identity::AuthContext;
use crate::auth::auth_key_lookup::tests::master_caller;

pub(crate) const SALT: &str = "test-salt-key";
pub(crate) const PLUGIN: &str = "litellm-platform-plugin";
// HMAC-SHA256("test-salt-key", "litellm-platform-plugin"), derived by Python's hmac
pub(crate) const KEY_HEX: &str = "f7fd53cdf1f945797b2766dae68ad55e76452f48e94d88699bfd805df0eaf472";
pub(crate) const CLAIM_TIMESTAMP: u64 = 1_700_000_000;
pub(crate) const CLAIM_IV: [u8; 16] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];
// json.dumps of the claim, sealed by Python's cryptography 43.0.0 with the key above
pub(crate) const CLAIM_PLAINTEXT: &str = r#"{"plugin": "litellm-platform-plugin", "user_id": "u1", "user_role": "proxy_admin", "exp": 1700000030}"#;
pub(crate) const CLAIM_TOKEN: &str = "gAAAAABlU_EAAAECAwQFBgcICQoLDA0ODyatsKjdJecKxMeGQ2JVCIKii4qlPgFigSEvFTArL79nA4ljojTDoyMy_fXCWRX10mkZ4C98SaCCQyMOhXCnkDI7vlvYgBggImrwXff4JNXHsw8KeQy3J-fSXexHoCnQQgRdds1URgP_HD_ZoVIiAM42btYDcuxip-X0-C_SlKQ1TfcpzCnLAzHNhsNkDQmfAA==";
// hmac.new("s" * 100, b"litellm-platform-plugin", sha256): the long key is hashed before padding
pub(crate) const LONG_SALT: &str = "ssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssss";
pub(crate) const LONG_SALT_KEY_HEX: &str =
    "9d5c17b150d61501068df2f03b0fabd04b1ba7a79b37c877563c54088a12c8e0";
pub(crate) const EMPTY_SALT_KEY_HEX: &str =
    "823db77a72224b45f13a2e82695e9f328beab0c39e5c4c736e83086be12e38c0";

fn auth_context(
    subject: &str,
    roles: Vec<Role>,
    user: Option<UserIdentity>,
    source_key: SourceKey,
) -> AuthContext {
    AuthContext {
        principal: Principal {
            principal_type: PrincipalType::ServiceAccount,
            subject: subject.to_owned(),
            issuer: None,
            audience: Vec::new(),
            user,
            organization: None,
            teams: Vec::new(),
            project: None,
            end_user: None,
            roles,
            scopes: Vec::new(),
            auth_method: AuthMethod::ApiKey,
            credential_ref: CredentialRef::default(),
            network: NetworkContext::default(),
            source_key,
        },
    }
}

pub(crate) fn caller(subject: &str, roles: Vec<Role>) -> AuthContext {
    auth_context(subject, roles, None, SourceKey::default())
}

pub(crate) fn master_key() -> AuthContext {
    master_caller()
}

pub(crate) fn user_caller(subject: &str, roles: Vec<Role>, user_id: &str) -> AuthContext {
    let user = UserIdentity {
        id: user_id.to_owned(),
        external_id: None,
        user_name: None,
        email: None,
        display_name: None,
    };
    auth_context(subject, roles, Some(user), SourceKey::default())
}

pub(crate) fn key_caller(fields: &str) -> AuthContext {
    let decoded: BTreeMap<String, JsonData> =
        serde_json::from_str(fields).expect("the key fields decode");
    let key =
        UserAPIKeyAuth::construct(AuthInput::Dict(decoded)).expect("the key fixture constructs");
    auth_context("key-owner", Vec::new(), None, SourceKey(Some(key)))
}

pub(crate) fn settings(json: &str) -> JsonObject {
    serde_json::from_str(json).expect("the settings decode")
}

pub(crate) fn registry(json: &str) -> Arc<PluginRegistry> {
    let registry = PluginRegistry::default();
    registry
        .replace_from_settings(&settings(json))
        .expect("the plugin settings load");
    Arc::new(registry)
}

pub(crate) fn state(
    registry: Arc<PluginRegistry>,
    upstream: Arc<dyn PluginUpstream>,
    key_header: Option<Arc<dyn KeyHeaderName>>,
) -> PluginState {
    PluginState {
        registry,
        upstream,
        key_header,
    }
}

pub(crate) async fn send(
    application: Router,
    request: Request<Body>,
) -> (StatusCode, HeaderMap, Vec<u8>) {
    let response = application
        .oneshot(request)
        .await
        .expect("the route answers");
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("the body is readable");
    (status, headers, bytes.to_vec())
}

pub(crate) fn authorized(mut request: Request<Body>, caller: AuthContext) -> Request<Body> {
    request.extensions_mut().insert(caller);
    request
}

pub(crate) fn text(bytes: &[u8]) -> String {
    String::from_utf8(bytes.to_vec()).expect("the body is UTF-8")
}

pub(crate) fn json(bytes: &[u8]) -> JsonValue {
    serde_json::from_slice(bytes).expect("the body is JSON")
}

pub(crate) fn reply(status: StatusCode, body: &str) -> PluginReply {
    PluginReply {
        status,
        headers: HeaderMap::new(),
        body: body.as_bytes().to_vec(),
    }
}

enum FakeReply {
    Answer(PluginReply),
    Failure,
}

#[derive(Default)]
pub(crate) struct FakeUpstream {
    requests: Mutex<Vec<PluginRequest>>,
    reply: Mutex<Option<FakeReply>>,
}

impl FakeUpstream {
    pub(crate) fn answering(reply: PluginReply) -> Arc<Self> {
        Arc::new(Self {
            requests: Mutex::new(Vec::new()),
            reply: Mutex::new(Some(FakeReply::Answer(reply))),
        })
    }

    pub(crate) fn failing() -> Arc<Self> {
        Arc::new(Self {
            requests: Mutex::new(Vec::new()),
            reply: Mutex::new(Some(FakeReply::Failure)),
        })
    }

    pub(crate) fn sent(&self) -> Vec<PluginRequest> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl PluginUpstream for FakeUpstream {
    async fn send(&self, request: PluginRequest) -> Result<PluginReply, PluginSendError> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
        let configured = self.reply.lock().unwrap_or_else(PoisonError::into_inner);
        match configured.as_ref() {
            Some(FakeReply::Answer(reply)) => Ok(reply.clone()),
            Some(FakeReply::Failure) | None => Err(PluginSendError::Transport {
                source: HttpError::Timeout,
            }),
        }
    }
}

#[derive(Default)]
pub(crate) struct KeyHeaderSource {
    name: Mutex<Option<String>>,
}

impl KeyHeaderSource {
    pub(crate) fn named(name: &str) -> Arc<Self> {
        Arc::new(Self {
            name: Mutex::new(Some(name.to_owned())),
        })
    }

    pub(crate) fn rename(&self, name: Option<&str>) {
        let mut held = self.name.lock().unwrap_or_else(PoisonError::into_inner);
        *held = name.map(str::to_owned);
    }
}

impl KeyHeaderName for KeyHeaderSource {
    fn litellm_key_header_name(&self) -> Option<String> {
        self.name
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}
