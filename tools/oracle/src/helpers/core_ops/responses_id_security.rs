use betterlite_auth::auth_identity::records::Principal;
use betterlite_auth::auth_identity::roles::Role;
use betterlite_config::sections::responses_id_security::Settings;
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::responses_id_security::ResponsesIdSecurity;
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_event_records::records::ResponsesAPIStreamingResponse;
use betterlite_openai::api::responses_objects::ResponsesApiResponse;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{Algorithm, CipherSettings, OsNonces, StoredCipher};
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};

mod ops;
mod tests;
mod values;

pub(crate) type Cipher = StoredCipher<OsNonces, TracingDiagnostics>;

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Authorize {
        settings: SettingsInput,
        principal: PrincipalInput,
        call_type: String,
        #[serde(default)]
        data: IndexMap<String, Json>,
    },
    Success {
        settings: SettingsInput,
        principal: PrincipalInput,
        // boxed so the response stays out of the enum's other variants
        response: Box<ResponsesApiResponse>,
        // the success hook takes no route; the case records one to show it stays unused
        #[serde(rename = "route")]
        _route: Option<String>,
    },
    Stream {
        settings: SettingsInput,
        principal: PrincipalInput,
        route: Option<String>,
        #[serde(default)]
        events: Vec<ResponsesAPIStreamingResponse>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SettingsInput {
    #[serde(default)]
    disable: bool,
    #[serde(default)]
    allow: bool,
    #[serde(default)]
    signing_key: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PrincipalInput {
    #[serde(default)]
    user_id: Option<String>,
    #[serde(default)]
    team_id: Option<String>,
    #[serde(default)]
    role: Option<String>,
    // the Python side picks the enum or the raw-value form; Rust reads the role the same way
    #[serde(rename = "role_form")]
    _role_form: Option<String>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Authorize {
                settings,
                principal,
                call_type,
                data,
            } => ops::authorize(settings, principal, call_type, data),
            Input::Success {
                settings,
                principal,
                response,
                _route,
            } => ops::success(settings, principal, response),
            Input::Stream {
                settings,
                principal,
                route,
                events,
            } => ops::stream(settings, principal, route.as_deref(), events),
        }
    }
}

pub(crate) struct Session {
    settings: Settings,
    cipher: Cipher,
}

impl Session {
    pub(crate) fn policy(&self) -> ResponsesIdSecurity<'_, OsNonces, TracingDiagnostics> {
        ResponsesIdSecurity {
            settings: &self.settings,
            cipher: &self.cipher,
            limits: ApiLimits::default(),
        }
    }
}

pub(crate) fn session(input: &SettingsInput) -> Session {
    let signing_key = input.signing_key.clone().map(SecretString::from);
    Session {
        settings: Settings {
            disable_responses_id_security: input.disable,
            allow_unmanaged_response_ids: input.allow,
            signing_key: signing_key.clone(),
        },
        cipher: StoredCipher {
            settings: CipherSettings {
                signing_key,
                algorithm: Algorithm::XSalsa20Poly1305,
            },
            nonces: OsNonces,
            diagnostics: TracingDiagnostics,
        },
    }
}

pub(crate) fn principal(input: &PrincipalInput) -> Result<Principal, Box<HelperError>> {
    let roles = match input.role.as_deref() {
        Some("PROXY_ADMIN") => vec![Role::PlatformAdmin],
        Some("PROXY_ADMIN_VIEW_ONLY") => vec![Role::PlatformViewer],
        Some(_) | None => Vec::new(),
    };
    let user = match &input.user_id {
        Some(id) => json!({ "id": id }),
        None => Value::Null,
    };
    let teams = match &input.team_id {
        Some(id) => json!([{ "id": id }]),
        None => json!([]),
    };
    let source = json!({
        "principal_type": "human",
        "subject": "subject",
        "auth_method": "api_key",
        "user": user,
        "teams": teams,
        "roles": roles,
    });
    serde_json::from_value(source).map_err(|error| failure("InvalidInput", error.to_string()))
}

pub(crate) fn failure(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
