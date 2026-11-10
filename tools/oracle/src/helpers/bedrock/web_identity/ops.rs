use super::cross::cross_account_irsa;
use super::fixture::{
    self, Answer, Record, Transport, assume_role_answer, denied_identity, web_identity_answer,
};
use super::input::{Case, Input, ReplyTag, WebIdentityReply};
use super::ports;
use crate::helpers::{HelperError, HelperResult};
use betterlite_bedrock::assume_role::expiry::{Expiration, StsResponse};
use betterlite_bedrock::aws_endpoints::error::AwsEndpointError;
use betterlite_bedrock::aws_endpoints::sts::StsClientKwargs;
use betterlite_bedrock::aws_settings::AwsSessionTag;
use betterlite_bedrock::aws_settings::sources::NativeSources;
use betterlite_bedrock::web_identity::auth::{WebIdentityAuth, auth_with_web_identity_token};
use betterlite_bedrock::web_identity::claims::decode_web_identity_claims;
use betterlite_bedrock::web_identity::error::WebIdentityError;
use betterlite_bedrock::web_identity::irsa::{SameAccountIrsa, handle_irsa_same_account};
use betterlite_config::sections::bedrock_web_identity::DEFAULT_MAX_POLICY_SIZE;
use secrecy::ExposeSecret as _;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use tokio::runtime::Runtime;
#[derive(Debug, thiserror::Error)]
pub(super) enum FixtureError {
    #[error(transparent)]
    Endpoint(#[from] AwsEndpointError),
    #[error(transparent)]
    Runtime(#[from] std::io::Error),
    #[error("{0}")]
    Case(&'static str),
}
impl From<FixtureError> for Box<HelperError> {
    fn from(error: FixtureError) -> Self {
        Self::new(HelperError {
            class: "FixtureError".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: error.to_string(),
        })
    }
}
pub(super) type Fixture = (NativeSources, Record, StsClientKwargs, Runtime);
pub(super) fn fixture(
    case: &Case,
    files: &[(&str, &str)],
    answers: BTreeMap<String, Answer>,
) -> Result<Fixture, Box<HelperError>> {
    let transport = Transport {
        answers,
        record: Record::default(),
    };
    let record = transport.record.clone();
    let native = fixture::sources(&case.environment, files, transport);
    let kwargs = fixture::kwargs(&case.environment).map_err(FixtureError::from)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(FixtureError::from)?;
    Ok((native, record, kwargs, runtime))
}
pub(super) fn run(input: &Input) -> HelperResult {
    match input {
        Input::JwtAudienceHint { tokens } => Ok(json!({
            "audiences": tokens
                .iter()
                .map(|token| decode_web_identity_claims(token))
                .collect::<Vec<Option<String>>>()
        })),
        Input::WebIdentity(case) => web_identity(case),
        Input::SameAccountIrsa(case) => same_account_irsa(case),
        Input::CrossAccountIrsa(case) => cross_account_irsa(case),
    }
}
pub(super) fn failure(error: &WebIdentityError) -> Box<HelperError> {
    let (class, status) = match error {
        WebIdentityError::Auth(auth) | WebIdentityError::RejectedToken { error: auth, .. } => {
            ("AwsAuthError", Some(auth.status))
        }
        WebIdentityError::Secret(_)
        | WebIdentityError::Endpoint(_)
        | WebIdentityError::TokenFile { .. }
        | WebIdentityError::TokenFileEncoding { .. }
        | WebIdentityError::MissingField(_)
        | WebIdentityError::WebIdentity { .. }
        | WebIdentityError::AssumeRole { .. }
        | WebIdentityError::SecretType
        | WebIdentityError::UnsupportedParameter { .. }
        | WebIdentityError::Policy { .. }
        | WebIdentityError::Tag { .. }
        | WebIdentityError::Expiry => ("WebIdentityError", None),
    };
    Box::new(HelperError {
        class: class.to_owned(),
        status,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
pub(super) fn tags_of(tags: &[ReplyTag]) -> Vec<AwsSessionTag> {
    tags.iter()
        .map(|tag| AwsSessionTag {
            key: tag.key.clone(),
            value: tag.value.clone(),
        })
        .collect()
}
pub(super) fn irsa_json(record: &Record, response: &StsResponse) -> Value {
    let credentials = &response.credentials;
    let expiration = match &credentials.expiration {
        Expiration::Aware(expiry) => expiry.timestamp(),
        Expiration::Naive(expiry) => expiry.and_utc().timestamp(),
    };
    json!({
        "calls": record.calls(),
        "credentials": [
            credentials.access_key_id.expose_secret(),
            credentials.secret_access_key.expose_secret(),
            credentials.session_token.expose_secret(),
        ],
        "expiration": expiration,
    })
}
fn web_identity(case: &Case) -> HelperResult {
    let Some(reply) = case.reply.as_ref() else {
        return Err(Box::<HelperError>::from(FixtureError::Case(
            "the case needs a web identity reply",
        )));
    };
    let answers = BTreeMap::from([(
        "AssumeRoleWithWebIdentity".to_owned(),
        web_identity_answer(reply),
    )]);
    let (native, record, kwargs, runtime) = fixture(case, &[], answers)?;
    let auth = WebIdentityAuth {
        token_reference: &case.token_reference,
        role_arn: &case.role_arn,
        session_name: &case.session_name,
        external_id: case.external_id.as_deref(),
        max_policy_size: case.max_policy_size.unwrap_or(DEFAULT_MAX_POLICY_SIZE),
    };
    let secrets = ports::secrets(case.resolved_token.as_deref());
    let (result, warnings) = ports::warnings(|| {
        runtime.block_on(auth_with_web_identity_token(
            &auth, &kwargs, &secrets, &native,
        ))
    });
    let credentials = result.map_err(|error| failure(&error))?;
    Ok(json!({
        "calls": record.calls(),
        "credentials": [
            credentials.credentials.access_key_id.expose_secret(),
            credentials.credentials.secret_access_key.expose_secret(),
            credentials.credentials.session_token.as_ref().map(secrecy::ExposeSecret::expose_secret),
        ],
        "ttl": credentials.cache_ttl.as_secs(),
        "warnings": warnings,
    }))
}
fn same_account_irsa(case: &Case) -> HelperResult {
    let Some(WebIdentityReply::Credentials(reply)) = case.reply.as_ref() else {
        return Err(Box::<HelperError>::from(FixtureError::Case(
            "the case needs a credentials reply",
        )));
    };
    let tags = tags_of(&case.tags);
    let input = SameAccountIrsa {
        role_arn: &case.role_arn,
        session_name: &case.session_name,
        external_id: case.external_id.as_deref(),
        tags: Some(tags.as_slice()),
    };
    let answers = BTreeMap::from([
        ("GetCallerIdentity".to_owned(), denied_identity()),
        ("AssumeRole".to_owned(), assume_role_answer(reply)),
    ]);
    let (native, record, kwargs, runtime) = fixture(case, &[], answers)?;
    let response = runtime
        .block_on(handle_irsa_same_account(&input, &kwargs, &native))
        .map_err(|error| failure(&error))?;
    Ok(irsa_json(&record, &response))
}
