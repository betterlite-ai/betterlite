use std::collections::BTreeMap;

use betterlite_bedrock::assume_role::expiry::{Expiration, StsResponse};
use betterlite_bedrock::web_identity::auth::{WebIdentityAuth, auth_with_web_identity_token};
use betterlite_bedrock::web_identity::claims::decode_web_identity_claims;
use betterlite_bedrock::web_identity::error::WebIdentityError;
use betterlite_bedrock::web_identity::irsa::{
    CrossAccountIrsa, SameAccountIrsa, handle_irsa_cross_account, handle_irsa_same_account,
};
use secrecy::ExposeSecret as _;
use serde_json::{Value, json};

use super::answers::{
    assume_role_answer, credentials_xml, identity_answer, tags_of, web_identity_answer,
};
use super::fixture::{self, Answer, Record, Transport};
use super::input::{CrossAccountCase, Input, SameAccountCase, WebIdentityCase, WebIdentityReply};
use super::ports;
use crate::helpers::{HelperError, HelperResult};

const DEFAULT_MAX_POLICY_SIZE: i64 = 75;

pub(super) fn run(input: &Input) -> HelperResult {
    match input {
        Input::JwtAudienceHint { tokens } | Input::MalformedToken { tokens } => {
            Ok(audiences(tokens))
        }
        Input::WebIdentityAssumeRole(case)
        | Input::SecureTransportSessionPolicy(case)
        | Input::InvalidIdentityTokenError(case)
        | Input::StsCredentialsMissing(case)
        | Input::PackedPolicyWarningAndTtl(case) => web_identity(case),
        Input::SameAccountIrsa(case) => same_account_irsa(case),
        Input::CrossAccountIrsa(case) => cross_account_irsa(case),
    }
}

fn audiences(tokens: &[String]) -> Value {
    let decoded: Vec<Option<String>> = tokens
        .iter()
        .map(|token| decode_web_identity_claims(token))
        .collect();
    json!({ "audiences": decoded })
}

fn failure(error: &WebIdentityError) -> Box<HelperError> {
    let (class, status) = match error {
        WebIdentityError::Auth(auth) | WebIdentityError::RejectedToken { error: auth, .. } => {
            ("AwsAuthError", Some(auth.status))
        }
        WebIdentityError::Secret(_)
        | WebIdentityError::Endpoint(_)
        | WebIdentityError::TokenFile { .. }
        | WebIdentityError::TokenFileEncoding { .. }
        | WebIdentityError::WebIdentityResponse { .. }
        | WebIdentityError::AssumeRoleResponse { .. }
        | WebIdentityError::WebIdentity { .. }
        | WebIdentityError::AssumeRole { .. }
        | WebIdentityError::SecretType { .. }
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

fn fixture_failure(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "FixtureError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn credentials_json(response: &StsResponse) -> Value {
    json!([
        response.credentials.access_key_id.expose_secret(),
        response.credentials.secret_access_key.expose_secret(),
        response.credentials.session_token.expose_secret(),
    ])
}

const fn expiration_epoch(response: &StsResponse) -> i64 {
    match &response.credentials.expiration {
        Expiration::Aware(expiry) => expiry.timestamp(),
        Expiration::Naive(expiry) => expiry.and_utc().timestamp(),
    }
}

fn web_identity_transport(reply: &WebIdentityReply) -> Transport {
    Transport {
        answers: BTreeMap::from([(
            "AssumeRoleWithWebIdentity".to_owned(),
            web_identity_answer(reply),
        )]),
        record: Record::default(),
    }
}

fn web_identity(case: &WebIdentityCase) -> HelperResult {
    let transport = web_identity_transport(&case.reply);
    let record = transport.record.clone();
    let native = fixture::sources(&case.environment, &[], transport);
    let secrets = ports::secrets(case.resolved_token.as_deref());
    let kwargs = fixture::kwargs(
        &case.environment,
        case.sts_endpoint.as_deref(),
        case.region_name.as_deref(),
        case.verify,
    )
    .map_err(fixture_failure)?;
    let auth = WebIdentityAuth {
        token_reference: &case.token_reference,
        role_arn: &case.role_arn,
        session_name: &case.session_name,
        external_id: case.external_id.as_deref(),
        max_policy_size: case.max_policy_size.unwrap_or(DEFAULT_MAX_POLICY_SIZE),
    };
    let runtime = ports::runtime().map_err(fixture_failure)?;
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

fn same_account_irsa(case: &SameAccountCase) -> HelperResult {
    let transport = Transport {
        answers: same_account_answers(case),
        record: Record::default(),
    };
    let record = transport.record.clone();
    let native = fixture::sources(&case.environment, &[], transport);
    let kwargs = fixture::kwargs(
        &case.environment,
        case.sts_endpoint.as_deref(),
        case.region_name.as_deref(),
        case.verify,
    )
    .map_err(fixture_failure)?;
    let tags = tags_of(&case.tags);
    let input = SameAccountIrsa {
        role_arn: &case.role_arn,
        session_name: &case.session_name,
        external_id: case.external_id.as_deref(),
        tags: Some(tags.as_slice()),
    };
    let runtime = ports::runtime().map_err(fixture_failure)?;
    let response = runtime
        .block_on(handle_irsa_same_account(&input, &kwargs, &native))
        .map_err(|error| failure(&error))?;
    Ok(json!({
        "calls": record.calls(),
        "credentials": credentials_json(&response),
        "expiration": expiration_epoch(&response),
    }))
}

fn same_account_answers(case: &SameAccountCase) -> BTreeMap<String, Answer> {
    BTreeMap::from([
        (
            "GetCallerIdentity".to_owned(),
            identity_answer(&case.identity),
        ),
        ("AssumeRole".to_owned(), assume_role_answer(&case.reply)),
    ])
}

fn cross_account_irsa(case: &CrossAccountCase) -> HelperResult {
    let transport = Transport {
        answers: cross_account_answers(case),
        record: Record::default(),
    };
    let record = transport.record.clone();
    let files = [(
        case.token_file.path.clone(),
        case.token_file.content.as_bytes().to_vec(),
    )];
    let native = fixture::sources(&case.environment, &files, transport);
    let kwargs = fixture::kwargs(
        &case.environment,
        case.sts_endpoint.as_deref(),
        case.region_name.as_deref(),
        case.verify,
    )
    .map_err(fixture_failure)?;
    let tags = tags_of(&case.tags);
    let input = CrossAccountIrsa {
        irsa_role_arn: &case.irsa_role_arn,
        role_arn: &case.role_arn,
        session_name: &case.session_name,
        token_file: &case.token_file.path,
        external_id: case.external_id.as_deref(),
        tags: Some(tags.as_slice()),
    };
    let runtime = ports::runtime().map_err(fixture_failure)?;
    let response = runtime
        .block_on(handle_irsa_cross_account(&input, &kwargs, &native))
        .map_err(|error| failure(&error))?;
    Ok(json!({
        "calls": record.calls(),
        "credentials": credentials_json(&response),
        "expiration": expiration_epoch(&response),
    }))
}

fn cross_account_answers(case: &CrossAccountCase) -> BTreeMap<String, Answer> {
    BTreeMap::from([
        (
            "AssumeRoleWithWebIdentity".to_owned(),
            Answer {
                status: 200,
                body: format!(
                    "<AssumeRoleWithWebIdentityResponse><AssumeRoleWithWebIdentityResult>{}</AssumeRoleWithWebIdentityResult></AssumeRoleWithWebIdentityResponse>",
                    credentials_xml(&case.irsa_reply)
                ),
            },
        ),
        (
            "GetCallerIdentity".to_owned(),
            identity_answer(&case.identity),
        ),
        ("AssumeRole".to_owned(), assume_role_answer(&case.reply)),
    ])
}
