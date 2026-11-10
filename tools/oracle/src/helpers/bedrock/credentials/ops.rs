use std::collections::BTreeMap;
use std::fmt::Display;

use betterlite_bedrock::aws_settings::sources::Credentials;
use betterlite_bedrock::credentials::AwsCredentialResolver;
use betterlite_bedrock::credentials::error::CredentialError;
use betterlite_bedrock::credentials::role::CredentialTtl;
use betterlite_bedrock::credentials::role::aws_role::auth_with_aws_role;
use betterlite_bedrock::web_identity::error::WebIdentityError;
use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::bedrock_aws_settings::AwsS3Settings;
use betterlite_config::sections::bedrock_settings::AwsSettings;
use secrecy::ExposeSecret;
use serde_json::{Map, Value, json};
use tokio::runtime::Builder;
use tracing::subscriber::with_default;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::registry;

use super::fixtures::{self, Call, Transport};
use super::input::{Case, DEFAULT_NOW, Input, Step};
use super::logs::Logs;
use super::role_input::OwnedRoleAuth;
use crate::helpers::{HelperError, HelperResult};

pub(super) fn failure<E>(error: E) -> Box<HelperError>
where
    E: Display,
{
    Box::new(HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

pub(super) fn run(input: &Input) -> HelperResult {
    match input {
        Input::GetCredentials(case) => run_case(case, Operation::GetCredentials),
        Input::ResolveCredentials(case) => run_case(case, Operation::ResolveCredentials),
        Input::ResolveS3Credentials(case) => run_case(case, Operation::ResolveS3Credentials),
        Input::AuthWithAwsRole(case) => run_case(case, Operation::AuthWithAwsRole),
    }
}

#[derive(Clone, Copy)]
enum Operation {
    GetCredentials,
    ResolveCredentials,
    ResolveS3Credentials,
    AuthWithAwsRole,
}

enum StepResult {
    Credentials {
        credentials: Credentials,
        ttl: Option<Value>,
    },
    Failed(Option<u16>),
}

fn run_case(case: &Case, operation: Operation) -> HelperResult {
    let transport = Transport::with(&case.answers);
    let native = fixtures::native(case, &transport);
    let secrets = fixtures::secrets(case.resolved_token.as_deref());
    let cache = fixtures::cache(case.now.unwrap_or(DEFAULT_NOW))?;
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(failure)?;
    let logs = Logs::default();
    let subscriber = registry().with(logs.clone());
    let steps = with_default(subscriber, || {
        case.steps
            .iter()
            .map(|step| {
                let recorded = transport.record.count();
                let result = fixtures::kwargs(&case.environment, &step.params).map_or_else(
                    |_| StepResult::Failed(None),
                    |sts| {
                        let resolver = AwsCredentialResolver {
                            cache: &cache,
                            native: &native,
                            secrets: &secrets,
                            sts: &sts,
                            max_policy_size: 75,
                        };
                        runtime.block_on(operation.run(&resolver, step, &case.environment))
                    },
                );
                let calls = transport.record.calls();
                step_json(result, calls.get(recorded..).unwrap_or_default())
            })
            .collect::<Vec<_>>()
    });
    Ok(json!({"steps": steps, "logs": logs.entries()}))
}

impl Operation {
    async fn run(
        &self,
        resolver: &AwsCredentialResolver<'_>,
        step: &Step,
        environment: &BTreeMap<String, String>,
    ) -> StepResult {
        if matches!(self, Self::AuthWithAwsRole) {
            let Ok(owned) = OwnedRoleAuth::parse(step, environment) else {
                return StepResult::Failed(None);
            };
            return match auth_with_aws_role(resolver, &owned.auth()).await {
                Ok(role) => StepResult::Credentials {
                    credentials: role.credentials,
                    ttl: role.ttl.map(|ttl| match ttl {
                        CredentialTtl::Fractional(seconds) => json!(seconds),
                        CredentialTtl::Whole(seconds) => json!(seconds),
                    }),
                },
                Err(error) => StepResult::Failed(status(&error)),
            };
        }
        let Ok((settings, s3)) = sections(step, environment) else {
            return StepResult::Failed(None);
        };
        let region = step.region.as_deref();
        match self {
            Self::GetCredentials => finish(
                resolver
                    .get_credentials(&settings, step.ssl_verify.as_ref())
                    .await,
            ),
            Self::ResolveCredentials => {
                finish(resolver.resolve_credentials(&settings, region).await)
            }
            Self::ResolveS3Credentials => finish(
                resolver
                    .resolve_s3_credentials(&s3, &settings, region)
                    .await,
            ),
            Self::AuthWithAwsRole => StepResult::Failed(None),
        }
    }
}

fn finish(result: Result<Credentials, CredentialError>) -> StepResult {
    match result {
        Ok(credentials) => StepResult::Credentials {
            credentials,
            ttl: None,
        },
        Err(error) => StepResult::Failed(status(&error)),
    }
}

fn sections(
    step: &Step,
    environment: &BTreeMap<String, String>,
) -> Result<(AwsSettings, AwsS3Settings), Box<HelperError>> {
    let mut params = Map::new();
    params.insert(
        "model".to_owned(),
        Value::String("bedrock/fixture".to_owned()),
    );
    for (name, value) in &step.params {
        params.insert(name.clone(), value.clone());
    }
    let source = format!(
        "model_list:\n- model_name: fixture\n  litellm_params: {}\n",
        Value::Object(params)
    );
    let config = ConfigLoader {
        lookup: environment.clone(),
    }
    .load(&source)
    .map_err(failure)?;
    let settings = config
        .sections
        .bedrock_settings
        .deployments
        .into_iter()
        .next()
        .ok_or_else(|| failure("the fixture config has no deployment"))?;
    let s3 = config
        .sections
        .bedrock_aws_settings
        .deployments
        .into_iter()
        .next()
        .ok_or_else(|| failure("the fixture config has no deployment"))?;
    Ok((settings, s3))
}

fn status(error: &CredentialError) -> Option<u16> {
    if let CredentialError::WebIdentity(WebIdentityError::Auth(auth)) = error {
        return Some(auth.status);
    }
    if let CredentialError::WebIdentity(WebIdentityError::RejectedToken { error: auth, .. }) = error
    {
        return Some(auth.status);
    }
    None
}

fn step_json(result: StepResult, calls: &[Call]) -> Value {
    match result {
        StepResult::Credentials { credentials, ttl } => json!({
            "ok": true,
            "credentials": [
                credentials.access_key_id.expose_secret(),
                credentials.secret_access_key.expose_secret(),
                credentials.session_token.as_ref().map(ExposeSecret::expose_secret),
            ],
            "ttl": ttl,
            "calls": calls,
        }),
        StepResult::Failed(status) => json!({
            "ok": false,
            "status": status,
            "calls": calls,
        }),
    }
}
