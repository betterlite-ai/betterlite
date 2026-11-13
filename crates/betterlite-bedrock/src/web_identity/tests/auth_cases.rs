use std::collections::BTreeMap;
use std::error::Error as _;
use std::sync::PoisonError;
use std::time::Duration;

use secrecy::ExposeSecret as _;

use super::fixture::{self, Answer, Exchange};
use crate::web_identity::auth::{
    WebIdentityAuth, WebIdentityCredentials, auth_with_web_identity_token,
};
use crate::web_identity::error::WebIdentityError;

const ENVIRONMENT: [(&str, &str); 4] = [
    ("AWS_ACCESS_KEY_ID", "AKIAAMBIENT"),
    ("AWS_SECRET_ACCESS_KEY", "ambient-secret"),
    ("AWS_SESSION_TOKEN", "ambient-session"),
    ("AWS_REGION", "us-east-1"),
];
const ROLE_ARN: &str = "arn:aws:iam::123456789012:role/TargetRole";
const SESSION_NAME: &str = "bedrock-session";
const TOKEN_REFERENCE: &str = "web-identity-token";
const RESOLVED_TOKEN: &str = "header.eyJhdWQiOiJsaXRlbGxtIn0.signature";
const ENDPOINT: &str = "https://sts.us-east-1.amazonaws.com";
const REGION: &str = "us-east-1";
const EXPIRATION: &str = "2026-01-01T01:00:00Z";
const ACCESS_KEY: &str = "ASIAFIXTURE";
const SECRET_KEY: &str = "fixture-secret";
const SESSION_TOKEN: &str = "fixture-session";
const TTL: Duration = Duration::from_mins(59);

pub(super) const EXPECTED_POLICY: &str = "{\"Version\":\"2012-10-17\",\"Statement\":[{\"Sid\":\"BedrockLiteLLM\",\"Effect\":\"Allow\",\"Action\":[\"bedrock:InvokeModel\",\"bedrock:InvokeModelWithResponseStream\",\"bedrock:CountTokens\",\"bedrock:Rerank\",\"bedrock:Retrieve\",\"bedrock:ListKnowledgeBases\",\"bedrock:InvokeAgent\",\"bedrock:ApplyGuardrail\",\"bedrock:GetGuardrail\",\"bedrock:ListGuardrails\"],\"Resource\":\"*\",\"Condition\":{\"Bool\":{\"aws:SecureTransport\":\"true\"}}},{\"Sid\":\"BedrockAgentCoreLiteLLM\",\"Effect\":\"Allow\",\"Action\":[\"bedrock-agentcore:InvokeAgentRuntime\",\"bedrock-agentcore:InvokeAgentRuntimeForUser\",\"bedrock-agentcore:InvokeGateway\"],\"Resource\":\"*\",\"Condition\":{\"Bool\":{\"aws:SecureTransport\":\"true\"}}},{\"Sid\":\"ClaudePlatformLiteLLM\",\"Effect\":\"Allow\",\"Action\":[\"aws-external-anthropic:CreateInference\",\"aws-external-anthropic:CreateBatchInference\",\"aws-external-anthropic:CancelBatchInference\",\"aws-external-anthropic:DeleteBatchInference\",\"aws-external-anthropic:CountTokens\",\"aws-external-anthropic:Get*\",\"aws-external-anthropic:List*\"],\"Resource\":\"*\",\"Condition\":{\"Bool\":{\"aws:SecureTransport\":\"true\"}}},{\"Sid\":\"BedrockMantleLiteLLM\",\"Effect\":\"Allow\",\"Action\":[\"bedrock-mantle:CreateInference\"],\"Resource\":\"*\",\"Condition\":{\"Bool\":{\"aws:SecureTransport\":\"true\"}}}]}";

struct Outcome {
    result: Result<WebIdentityCredentials, WebIdentityError>,
    exchanges: Vec<Exchange>,
}

fn run(reference: &str, resolved: Option<&str>, max_policy_size: i64, answer: Answer) -> Outcome {
    run_with(
        reference,
        resolved,
        None,
        max_policy_size,
        BTreeMap::from([("AssumeRoleWithWebIdentity".to_owned(), answer)]),
    )
}

fn run_with(
    reference: &str,
    resolved: Option<&str>,
    external_id: Option<&str>,
    max_policy_size: i64,
    replies: BTreeMap<String, Answer>,
) -> Outcome {
    let (native, recorder) = fixture::sources(&ENVIRONMENT, &[], replies);
    let secrets = fixture::secrets(resolved);
    let kwargs = fixture::kwargs(&ENVIRONMENT, Some(ENDPOINT), Some(REGION));
    let auth = WebIdentityAuth {
        token_reference: reference,
        role_arn: ROLE_ARN,
        session_name: SESSION_NAME,
        external_id,
        max_policy_size,
    };
    let result = fixture::runtime().block_on(auth_with_web_identity_token(
        &auth, &kwargs, &secrets, &native,
    ));
    let exchanges = recorder
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    Outcome { result, exchanges }
}

fn ok_answer(packed_policy_size: Option<i32>) -> Answer {
    fixture::web_identity_ok(
        ACCESS_KEY,
        SECRET_KEY,
        SESSION_TOKEN,
        EXPIRATION,
        packed_policy_size,
    )
}

fn form<'exchange>(exchange: &'exchange Exchange, name: &str) -> Option<&'exchange str> {
    exchange.form.get(name).map(String::as_str)
}

fn status(error: &WebIdentityError) -> Option<u16> {
    if let WebIdentityError::Auth(auth) = &error {
        return Some(auth.status);
    }
    if let WebIdentityError::RejectedToken { error: auth, .. } = &error {
        return Some(auth.status);
    }
    None
}

pub(super) fn web_identity_assume_role() {
    let outcome = run(
        TOKEN_REFERENCE,
        Some(RESOLVED_TOKEN),
        75_i64,
        ok_answer(Some(1_i32)),
    );
    let credentials = outcome.result.unwrap();
    assert_credentials(&credentials);
    assert_eq!(outcome.exchanges.len(), 1_usize);
    assert_web_identity_call(&outcome.exchanges[0]);
    assert_empty_token_forwarded();
    assert_external_id_rejected();
    assert_token_rejections();
}

fn assert_empty_token_forwarded() {
    let outcome = run(TOKEN_REFERENCE, Some(""), 75_i64, ok_answer(Some(1_i32)));
    outcome.result.unwrap();
    assert_eq!(outcome.exchanges.len(), 1_usize);
    assert_eq!(form(&outcome.exchanges[0], "WebIdentityToken"), Some(""));
}

fn assert_credentials(credentials: &WebIdentityCredentials) {
    assert_eq!(
        credentials.credentials.access_key_id.expose_secret(),
        ACCESS_KEY
    );
    assert_eq!(
        credentials.credentials.secret_access_key.expose_secret(),
        SECRET_KEY
    );
    assert_eq!(
        credentials
            .credentials
            .session_token
            .as_ref()
            .map(secrecy::ExposeSecret::expose_secret),
        Some(SESSION_TOKEN)
    );
    assert_eq!(credentials.cache_ttl, TTL);
}

fn assert_web_identity_call(exchange: &Exchange) {
    assert_eq!(exchange.action, "AssumeRoleWithWebIdentity");
    assert_eq!(form(exchange, "RoleArn"), Some(ROLE_ARN));
    assert_eq!(form(exchange, "RoleSessionName"), Some(SESSION_NAME));
    assert_eq!(form(exchange, "WebIdentityToken"), Some(RESOLVED_TOKEN));
    assert_eq!(form(exchange, "DurationSeconds"), Some("3600"));
    assert_eq!(form(exchange, "Policy"), Some(EXPECTED_POLICY));
    assert_eq!(form(exchange, "ExternalId"), None);
    assert!(exchange.form.keys().all(|name| !name.starts_with("Tags")));
    assert!(exchange.access_key.is_none());
    assert!(exchange.session_token.is_none());
    assert!(exchange.region.is_none());
    assert_eq!(
        exchange.host.as_deref(),
        Some("sts.us-east-1.amazonaws.com")
    );
}

fn assert_external_id_rejected() {
    for external_id in ["", "external-1"] {
        let outcome = run_with(
            TOKEN_REFERENCE,
            Some(RESOLVED_TOKEN),
            Some(external_id),
            75_i64,
            BTreeMap::from([(
                "AssumeRoleWithWebIdentity".to_owned(),
                ok_answer(Some(1_i32)),
            )]),
        );
        assert!(matches!(
            outcome.result.unwrap_err(),
            WebIdentityError::UnsupportedParameter {
                field: "aws_external_id"
            }
        ));
        assert!(outcome.exchanges.is_empty());
    }
}

fn assert_token_rejections() {
    let rejected = run(
        "os.environ/TOKEN",
        Some(RESOLVED_TOKEN),
        75_i64,
        ok_answer(Some(1_i32)),
    );
    assert_eq!(status(&rejected.result.unwrap_err()), Some(400_u16));
    assert!(rejected.exchanges.is_empty());
    let environment = run(
        "AWS_ACCESS_KEY_ID",
        Some(RESOLVED_TOKEN),
        75_i64,
        ok_answer(Some(1_i32)),
    );
    assert_eq!(status(&environment.result.unwrap_err()), Some(400_u16));
    assert!(environment.exchanges.is_empty());
    let missing = run(TOKEN_REFERENCE, None, 75_i64, ok_answer(Some(1_i32)));
    assert_eq!(status(&missing.result.unwrap_err()), Some(401_u16));
    assert!(missing.exchanges.is_empty());
}

pub(super) fn secure_transport_session_policy() {
    let outcome = run(
        TOKEN_REFERENCE,
        Some(RESOLVED_TOKEN),
        75_i64,
        ok_answer(Some(1_i32)),
    );
    outcome.result.unwrap();
    let policy = form(&outcome.exchanges[0], "Policy").unwrap();
    assert_eq!(policy, EXPECTED_POLICY);
    for statement in [
        "\"Sid\":\"BedrockLiteLLM\"",
        "\"Sid\":\"BedrockAgentCoreLiteLLM\"",
        "\"Sid\":\"ClaudePlatformLiteLLM\"",
        "\"Sid\":\"BedrockMantleLiteLLM\"",
        "\"Effect\":\"Allow\"",
        "\"Resource\":\"*\"",
        "\"aws:SecureTransport\":\"true\"",
        "\"Version\":\"2012-10-17\"",
    ] {
        assert!(policy.contains(statement), "{statement}");
    }
}

pub(super) fn sts_credentials_missing() {
    let outcome = run(
        TOKEN_REFERENCE,
        Some(RESOLVED_TOKEN),
        75_i64,
        fixture::web_identity_without_credentials(Some(1_i32)),
    );
    assert!(matches!(
        outcome.result.unwrap_err(),
        WebIdentityError::MissingField(message) if message.ends_with("no Credentials")
    ));
    assert_missing_packed_policy_size();
    assert_propagated_failures();
}

fn assert_missing_packed_policy_size() {
    let outcome = run(
        TOKEN_REFERENCE,
        Some(RESOLVED_TOKEN),
        75_i64,
        fixture::web_identity_ok(ACCESS_KEY, SECRET_KEY, SESSION_TOKEN, EXPIRATION, None),
    );
    assert!(matches!(
        outcome.result.unwrap_err(),
        WebIdentityError::MissingField(message) if message.ends_with("no PackedPolicySize")
    ));
}

fn assert_propagated_failures() {
    let incomplete = Answer::Ok("<AssumeRoleWithWebIdentityResponse><AssumeRoleWithWebIdentityResult><Credentials><SecretAccessKey>s</SecretAccessKey><SessionToken>t</SessionToken><Expiration>2026-01-01T01:00:00Z</Expiration></Credentials><PackedPolicySize>1</PackedPolicySize></AssumeRoleWithWebIdentityResult></AssumeRoleWithWebIdentityResponse>".to_owned());
    let outcome = run(TOKEN_REFERENCE, Some(RESOLVED_TOKEN), 75_i64, incomplete);
    assert!(matches!(
        outcome.result.unwrap_err(),
        WebIdentityError::MissingField(message) if message.ends_with("no AccessKeyId")
    ));
    assert_missing_members();
    let denied = run(
        TOKEN_REFERENCE,
        Some(RESOLVED_TOKEN),
        75_i64,
        fixture::access_denied(),
    );
    assert!(matches!(
        denied.result.unwrap_err(),
        WebIdentityError::WebIdentity { .. }
    ));
}

fn assert_missing_members() {
    for member in ["AccessKeyId", "SecretAccessKey", "SessionToken"] {
        let outcome = run(
            TOKEN_REFERENCE,
            Some(RESOLVED_TOKEN),
            75_i64,
            fixture::web_identity_missing(member),
        );
        fixture::assert_missing(&outcome.result.unwrap_err(), member);
    }
}

pub(super) fn invalid_identity_token_error() {
    let outcome = run(
        TOKEN_REFERENCE,
        Some(RESOLVED_TOKEN),
        75_i64,
        fixture::invalid_identity_token(),
    );
    let error = outcome.result.unwrap_err();
    let WebIdentityError::RejectedToken {
        error: rejected,
        source,
    } = &error
    else {
        panic!("expected a rejected token, got {error:?}");
    };
    assert_eq!(rejected.status, 401_u16);
    assert!(
        rejected
            .message
            .starts_with("AWS STS rejected the web identity token: "),
        "{}",
        rejected.message
    );
    assert!(
        rejected
            .message
            .ends_with(". Token aud='litellm', iss=None"),
        "{}",
        rejected.message
    );
    assert!(source.source().is_some());
    assert_plain_rejection();
    assert_other_sts_error_propagates();
}

fn assert_plain_rejection() {
    let outcome = run(
        TOKEN_REFERENCE,
        Some("header..signature"),
        75_i64,
        fixture::invalid_identity_token(),
    );
    let WebIdentityError::RejectedToken {
        error: rejected, ..
    } = outcome.result.unwrap_err()
    else {
        panic!("expected a rejected token");
    };
    assert_eq!(rejected.status, 401_u16);
    assert!(
        !rejected.message.contains(" Token "),
        "{}",
        rejected.message
    );
    assert!(rejected.message.ends_with('.'), "{}", rejected.message);
}

fn assert_other_sts_error_propagates() {
    let outcome = run(
        TOKEN_REFERENCE,
        Some(RESOLVED_TOKEN),
        75_i64,
        fixture::access_denied(),
    );
    assert!(matches!(
        outcome.result.unwrap_err(),
        WebIdentityError::WebIdentity { .. }
    ));
}

pub(super) fn packed_policy_warning_and_ttl() {
    assert_warning_above_threshold();
    assert_no_warning_at_threshold();
    assert_default_threshold();
}

fn assert_warning_above_threshold() {
    let captured = fixture::warnings(|| {
        let outcome = run(
            TOKEN_REFERENCE,
            Some(RESOLVED_TOKEN),
            10_i64,
            ok_answer(Some(11_i32)),
        );
        assert_eq!(outcome.result.unwrap().cache_ttl, TTL);
    });
    assert_eq!(
        captured,
        [
            "The policy size is greater than 75% of the allowed size, PackedPolicySize: 11"
                .to_owned()
        ]
    );
}

fn assert_no_warning_at_threshold() {
    for (threshold, packed) in [
        (10_i64, 10_i32),
        (75_i64, 75_i32),
        (75_i64, 1_i32),
        (75_i64, -5_i32),
    ] {
        let captured = fixture::warnings(|| {
            let outcome = run(
                TOKEN_REFERENCE,
                Some(RESOLVED_TOKEN),
                threshold,
                ok_answer(Some(packed)),
            );
            outcome.result.unwrap();
        });
        assert!(captured.is_empty(), "{threshold} against {packed}");
    }
}

fn assert_default_threshold() {
    let captured = fixture::warnings(|| {
        let outcome = run(
            TOKEN_REFERENCE,
            Some(RESOLVED_TOKEN),
            75_i64,
            ok_answer(Some(76_i32)),
        );
        outcome.result.unwrap();
    });
    assert_eq!(
        captured,
        [
            "The policy size is greater than 75% of the allowed size, PackedPolicySize: 76"
                .to_owned()
        ]
    );
    let far = run(
        TOKEN_REFERENCE,
        Some(RESOLVED_TOKEN),
        75_i64,
        fixture::web_identity_ok(
            ACCESS_KEY,
            SECRET_KEY,
            SESSION_TOKEN,
            "2030-06-01T12:00:00Z",
            Some(1_i32),
        ),
    );
    assert_eq!(far.result.unwrap().cache_ttl, TTL);
}
