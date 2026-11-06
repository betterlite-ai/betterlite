#![cfg(test)]

use std::collections::BTreeMap;
use std::fmt::Debug;
use std::future::ready;
use std::io;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use thiserror::Error;
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber, subscriber::with_default};
use tracing_subscriber::layer::{Context, SubscriberExt as _};
use tracing_subscriber::{Layer, registry};

use crate::aws::client_init::{
    AwsKeyManagementSettings, AwsSecretManagerRuntime, load_aws_secret_manager,
    validate_environment,
};
use crate::aws::request_signing::{
    AwsCredentialsInfo, AwsOptionalParams, AwsTransport, CredentialResolver, TransportError,
    TransportFuture, TransportRequest,
};
use crate::aws::{AwsDependencies, AwsSecretsManager};
use crate::secret_types::KeyManagementSystem;

struct NoResolver;

impl CredentialResolver for NoResolver {
    type Error = RuntimeFailure;

    fn resolve_credentials(
        &self,
        _optional_params: &mut AwsOptionalParams,
    ) -> Result<AwsCredentialsInfo, RuntimeFailure> {
        Err(RuntimeFailure::Client)
    }
}

#[derive(Debug, Error)]
enum RuntimeFailure {
    #[error("the client could not be registered")]
    Client,
    #[error("the key management system could not be registered")]
    System,
}

#[derive(Default)]
struct Runtime {
    client: Option<AwsSecretsManager<NoResolver, NoTransport>>,
    system: Option<KeyManagementSystem>,
    fail_client: bool,
    fail_system: bool,
}

impl AwsSecretManagerRuntime<NoResolver, NoTransport> for Runtime {
    type Error = RuntimeFailure;

    fn set_secret_manager_client(
        &mut self,
        client: AwsSecretsManager<NoResolver, NoTransport>,
    ) -> Result<(), RuntimeFailure> {
        if self.fail_client {
            return Err(RuntimeFailure::Client);
        }
        self.client = Some(client);
        Ok(())
    }

    fn set_key_management_system(
        &mut self,
        system: KeyManagementSystem,
    ) -> Result<(), RuntimeFailure> {
        if self.fail_system {
            return Err(RuntimeFailure::System);
        }
        self.system = Some(system);
        Ok(())
    }
}

#[derive(Clone, Default)]
struct Warnings(Arc<Mutex<Vec<String>>>);

struct Message<'collected>(&'collected mut String);

impl Visit for Message<'_> {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            *self.0 = format!("{value:?}");
        }
    }
}

impl<Inner> Layer<Inner> for Warnings
where
    Inner: Subscriber,
{
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        if *event.metadata().level() == Level::WARN {
            let mut text = String::new();
            event.record(&mut Message(&mut text));
            self.0.lock().unwrap().push(text);
        }
    }
}

#[derive(Debug)]
struct NoTransport;

impl AwsTransport for NoTransport {
    type Error = io::Error;

    fn post<'request>(
        &'request self,
        _request: TransportRequest<'request>,
    ) -> TransportFuture<'request, io::Error> {
        Box::pin(ready(Err(TransportError::Timeout)))
    }
}

fn environment(names: &[&str]) -> BTreeMap<String, String> {
    names
        .iter()
        .map(|name| ((*name).to_owned(), "us-east-1".to_owned()))
        .collect()
}

fn dependencies(names: &[&str]) -> AwsDependencies<NoResolver, NoTransport> {
    AwsDependencies {
        environment: environment(names),
        transport: Arc::new(NoTransport),
        resolver: NoResolver,
        dns_suffix: |_region| "amazonaws.com",
        clock: SystemTime::now,
    }
}

fn full_settings() -> AwsKeyManagementSettings {
    AwsKeyManagementSettings {
        aws_region_name: Some("eu-west-1".to_owned()),
        aws_role_name: Some("arn:aws:iam::123456789012:role/Reader".to_owned()),
        aws_session_name: Some("session".to_owned()),
        aws_external_id: Some("external".to_owned()),
        aws_profile_name: Some("profile".to_owned()),
        aws_web_identity_token: Some(SecretString::from("web-token")),
        aws_sts_endpoint: Some("https://sts.example.test".to_owned()),
        replica_regions: vec!["us-west-2".to_owned(), "ap-south-1".to_owned()],
        kms_key_id: Some("alias/secrets".to_owned()),
    }
}

fn load_and_collect_warnings(
    flag: bool,
    settings: &AwsKeyManagementSettings,
    environment_names: &[&str],
    runtime: &mut Runtime,
) -> (Result<(), RuntimeFailure>, Vec<String>) {
    let warnings = Warnings::default();
    let subscriber = registry().with(warnings.clone());
    let outcome = with_default(subscriber, || {
        load_aws_secret_manager(flag, settings, dependencies(environment_names), runtime)
    });
    let collected = warnings.0.lock().unwrap().clone();
    (outcome, collected)
}

mod aws_secrets_manager_tests_behavior {
    use super::*;

    #[test]
    fn load_registers_nothing_unless_the_flag_is_true() {
        let mut runtime = Runtime::default();
        let (outcome, warnings) =
            load_and_collect_warnings(false, &full_settings(), &[], &mut runtime);
        outcome.unwrap();
        assert!(runtime.client.is_none());
        assert!(runtime.system.is_none());
        assert!(warnings.is_empty());
    }

    #[test]
    fn load_stores_every_key_management_setting_on_the_client() {
        let mut runtime = Runtime::default();
        let (outcome, _warnings) =
            load_and_collect_warnings(true, &full_settings(), &["AWS_REGION_NAME"], &mut runtime);
        outcome.unwrap();
        let client = runtime.client.unwrap();
        assert_eq!(client.aws_region_name.as_deref(), Some("eu-west-1"));
        assert_eq!(
            client.aws_role_name.as_deref(),
            Some("arn:aws:iam::123456789012:role/Reader")
        );
        assert_eq!(client.aws_session_name.as_deref(), Some("session"));
        assert_eq!(client.aws_external_id.as_deref(), Some("external"));
        assert_eq!(client.aws_profile_name.as_deref(), Some("profile"));
        assert_eq!(
            client.aws_web_identity_token.unwrap().expose_secret(),
            "web-token"
        );
        assert_eq!(
            client.aws_sts_endpoint.as_deref(),
            Some("https://sts.example.test")
        );
        assert_eq!(client.replica_regions, ["us-west-2", "ap-south-1"]);
        assert_eq!(client.kms_key_id.as_deref(), Some("alias/secrets"));
        assert_eq!(runtime.system, Some(KeyManagementSystem::AwsSecretManager));
    }

    #[test]
    fn load_without_settings_builds_a_client_with_empty_settings() {
        let mut runtime = Runtime::default();
        let (outcome, _warnings) = load_and_collect_warnings(
            true,
            &AwsKeyManagementSettings::default(),
            &["AWS_REGION"],
            &mut runtime,
        );
        outcome.unwrap();
        let client = runtime.client.unwrap();
        assert!(client.aws_region_name.is_none());
        assert!(client.aws_role_name.is_none());
        assert!(client.aws_session_name.is_none());
        assert!(client.aws_external_id.is_none());
        assert!(client.aws_profile_name.is_none());
        assert!(client.aws_web_identity_token.is_none());
        assert!(client.aws_sts_endpoint.is_none());
        assert!(client.replica_regions.is_empty());
        assert!(client.kms_key_id.is_none());
        assert_eq!(runtime.system, Some(KeyManagementSystem::AwsSecretManager));
    }

    #[test]
    fn load_keeps_settings_as_given_without_validating_them() {
        let settings = AwsKeyManagementSettings {
            aws_region_name: Some(String::new()),
            aws_role_name: Some("not an arn".to_owned()),
            aws_session_name: None,
            aws_external_id: None,
            aws_profile_name: None,
            aws_web_identity_token: None,
            aws_sts_endpoint: None,
            replica_regions: Vec::new(),
            kms_key_id: None,
        };
        let mut runtime = Runtime::default();
        let (outcome, _warnings) =
            load_and_collect_warnings(true, &settings, &["AWS_DEFAULT_REGION"], &mut runtime);
        outcome.unwrap();
        let client = runtime.client.unwrap();
        assert_eq!(client.aws_region_name.as_deref(), Some(""));
        assert_eq!(client.aws_role_name.as_deref(), Some("not an arn"));
    }

    #[test]
    fn load_hands_the_injected_environment_to_the_client() {
        let mut runtime = Runtime::default();
        let (outcome, _warnings) = load_and_collect_warnings(
            true,
            &AwsKeyManagementSettings::default(),
            &["AWS_REGION_NAME", "AWS_BEDROCK_RUNTIME_ENDPOINT"],
            &mut runtime,
        );
        outcome.unwrap();
        let client = runtime.client.unwrap();
        assert_eq!(
            client.dependencies.environment,
            environment(&["AWS_REGION_NAME", "AWS_BEDROCK_RUNTIME_ENDPOINT"])
        );
    }

    #[rstest]
    #[case::client(true, false, "the client could not be registered")]
    #[case::system(false, true, "the key management system could not be registered")]
    fn load_surfaces_runtime_failures(
        #[case] fail_client: bool,
        #[case] fail_system: bool,
        #[case] message: &str,
    ) {
        let mut runtime = Runtime {
            client: None,
            system: None,
            fail_client,
            fail_system,
        };
        let (outcome, _warnings) = load_and_collect_warnings(
            true,
            &AwsKeyManagementSettings::default(),
            &["AWS_REGION"],
            &mut runtime,
        );
        assert_eq!(outcome.unwrap_err().to_string(), message);
    }

    #[test]
    fn a_failed_client_registration_leaves_the_key_management_system_unset() {
        let mut runtime = Runtime {
            client: None,
            system: None,
            fail_client: true,
            fail_system: false,
        };
        let (_outcome, _warnings) = load_and_collect_warnings(
            true,
            &AwsKeyManagementSettings::default(),
            &["AWS_REGION"],
            &mut runtime,
        );
        assert!(runtime.system.is_none());
    }

    #[test]
    fn validate_environment_warns_when_no_region_variable_exists() {
        let warnings = Warnings::default();
        let subscriber = registry().with(warnings.clone());
        with_default(subscriber, || validate_environment(&environment(&[])));
        let collected = warnings.0.lock().unwrap().clone();
        assert_eq!(
            collected,
            [
                "No AWS region found in environment. Ensure aws_region_name is set in key_management_settings or AWS_REGION_NAME/AWS_REGION/AWS_DEFAULT_REGION is set in environment."
            ]
        );
    }

    #[rstest]
    #[case::region_name(&["AWS_REGION_NAME"])]
    #[case::region(&["AWS_REGION"])]
    #[case::default_region(&["AWS_DEFAULT_REGION"])]
    #[case::all_three(&["AWS_REGION_NAME", "AWS_REGION", "AWS_DEFAULT_REGION"])]
    fn validate_environment_stays_quiet_when_any_region_variable_exists(#[case] names: &[&str]) {
        let warnings = Warnings::default();
        let subscriber = registry().with(warnings.clone());
        with_default(subscriber, || validate_environment(&environment(names)));
        assert!(warnings.0.lock().unwrap().is_empty());
    }

    #[test]
    fn a_region_variable_with_an_empty_value_still_counts_as_present() {
        let environment = BTreeMap::from([("AWS_REGION".to_owned(), String::new())]);
        let warnings = Warnings::default();
        let subscriber = registry().with(warnings.clone());
        with_default(subscriber, || validate_environment(&environment));
        assert!(warnings.0.lock().unwrap().is_empty());
    }

    #[test]
    fn load_warns_about_a_missing_region_and_still_registers_the_client() {
        let mut runtime = Runtime::default();
        let (outcome, warnings) = load_and_collect_warnings(
            true,
            &AwsKeyManagementSettings::default(),
            &["AWS_ACCESS_KEY_ID"],
            &mut runtime,
        );
        outcome.unwrap();
        assert_eq!(warnings.len(), 1);
        assert!(runtime.client.is_some());
        assert_eq!(runtime.system, Some(KeyManagementSystem::AwsSecretManager));
    }

    #[test]
    fn a_disabled_loader_does_not_check_the_environment() {
        let mut runtime = Runtime::default();
        let (outcome, warnings) = load_and_collect_warnings(
            false,
            &AwsKeyManagementSettings::default(),
            &[],
            &mut runtime,
        );
        outcome.unwrap();
        assert!(warnings.is_empty());
    }
}
