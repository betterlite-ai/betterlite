use std::future::Future;
use std::sync::Mutex;

use betterlite_auth::auth_identity::records::{
    CredentialRef, EndUserIdentity, NetworkContext, OrganizationIdentity, Principal, SourceKey,
    TeamIdentity, UserIdentity,
};
use betterlite_auth::auth_identity::roles::{AuthMethod, PrincipalType, TeamRole};
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_core::passthrough_guardrail::wire::parse_request_data;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_base::contract::GuardrailContract;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};
use betterlite_guardrails::guardrail_message_scope::Object;

use crate::guardrails::chat_guardrail_translation::rewrites::response::ChatResponse;

pub(crate) type Reply = Box<dyn Fn(GuardrailApiInputs) -> GuardrailApiInputs + Send + Sync>;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Recorded {
    pub(crate) inputs: GuardrailApiInputs,
    pub(crate) input_type: GuardrailInputType,
    pub(crate) request_data: Object,
}

pub(crate) struct FakeGuardrail {
    reply: Reply,
    failure: Option<GuardrailFailure>,
    covers_full_request: bool,
    calls: Mutex<Vec<Recorded>>,
}

impl FakeGuardrail {
    pub(crate) fn answering(reply: Reply) -> Self {
        Self {
            reply,
            failure: None,
            covers_full_request: false,
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn passing_through() -> Self {
        Self::answering(Box::new(|inputs| inputs))
    }

    pub(crate) fn failing(failure: GuardrailFailure) -> Self {
        Self {
            reply: Box::new(|inputs| inputs),
            failure: Some(failure),
            covers_full_request: false,
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn covering_full_request(mut self) -> Self {
        self.covers_full_request = true;
        self
    }

    pub(crate) fn calls(&self) -> Vec<Recorded> {
        self.calls.lock().unwrap().clone()
    }
}

impl GuardrailContract for FakeGuardrail {
    fn apply_guardrail(
        &self,
        inputs: GuardrailApiInputs,
        request_data: &mut Object,
        input_type: GuardrailInputType,
        _logging_obj: Option<&mut GuardrailLogTarget>,
    ) -> impl Future<Output = Result<GuardrailApiInputs, GuardrailFailure>> + Send {
        self.calls.lock().unwrap().push(Recorded {
            inputs: inputs.clone(),
            input_type,
            request_data: request_data.clone(),
        });
        let outcome = self
            .failure
            .as_ref()
            .map_or_else(|| Ok((self.reply)(inputs)), |failure| Err(failure.clone()));
        async move { outcome }
    }

    fn messages_cover_full_request(&self) -> bool {
        self.covers_full_request
    }
}

pub(crate) fn object(source: &str) -> Object {
    parse_request_data(source).unwrap()
}

pub(crate) fn values(source: &str) -> Vec<GuardrailValue> {
    match object(&format!(r#"{{"items": {source}}}"#)).remove("items") {
        Some(GuardrailValue::Array(items)) => items,
        other => panic!("expected a JSON array, got {other:?}"),
    }
}

pub(crate) fn chunks(source: &str) -> Vec<Object> {
    values(source)
        .into_iter()
        .map(|chunk| match chunk {
            GuardrailValue::Object(fields) => fields,
            other @ (GuardrailValue::Null
            | GuardrailValue::Boolean(_)
            | GuardrailValue::SignedInteger(_)
            | GuardrailValue::UnsignedInteger(_)
            | GuardrailValue::Decimal(_)
            | GuardrailValue::Text(_)
            | GuardrailValue::Array(_)) => panic!("expected an object, got {other:?}"),
        })
        .collect()
}

pub(crate) fn response(source: &str) -> ChatResponse {
    serde_json::from_str(source).unwrap()
}

pub(crate) fn principal() -> Principal {
    Principal {
        principal_type: PrincipalType::ServiceAccount,
        subject: "tester".to_owned(),
        issuer: None,
        audience: Vec::new(),
        user: Some(UserIdentity {
            id: "user-1".to_owned(),
            external_id: None,
            user_name: None,
            email: Some("user@example.com".to_owned()),
            display_name: None,
        }),
        organization: Some(OrganizationIdentity {
            id: "org-1".to_owned(),
            name: None,
        }),
        teams: vec![TeamIdentity {
            id: "team-1".to_owned(),
            name: Some("Platform".to_owned()),
            role: TeamRole::Member,
        }],
        project: None,
        end_user: Some(EndUserIdentity {
            id: "end-1".to_owned(),
        }),
        roles: Vec::new(),
        scopes: Vec::new(),
        auth_method: AuthMethod::ApiKey,
        credential_ref: CredentialRef {
            key_id: None,
            token_id: Some("hash-1".to_owned()),
        },
        network: NetworkContext::default(),
        source_key: SourceKey::default(),
    }
}
