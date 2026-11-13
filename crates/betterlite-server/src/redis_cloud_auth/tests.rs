#![cfg(test)]

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use aws_credential_types::Credentials;
use aws_credential_types::credential_fn::provide_credentials_fn;
use aws_credential_types::provider::SharedCredentialsProvider;
use aws_credential_types::provider::error::CredentialsError;
use betterlite_azure::azure_certificate::assertion::RandomIds;
use betterlite_azure::azure_certificate::credential::StdFiles;
use betterlite_azure::azure_client_secret::provider::ProviderCache;
use betterlite_azure::azure_default_chain::environment::ChainEnvironment;
use betterlite_azure::azure_managed_identity::environment::ManagedIdentityEnvironment;
use betterlite_azure::azure_managed_identity::runtime::FsTokenFiles;
use betterlite_azure::oidc_exchange::AzureOidcExchange;
use betterlite_config::sections::azure_client_secret::Settings as ClientSecretSettings;
use betterlite_config::sections::azure_oidc_exchange::Settings as OidcSettings;
use betterlite_config::sections::redis_credentials::Settings;
use betterlite_config::sections::redis_credentials::aws::{
    AwsCredentialsResolver, ElastiCacheIamProvider, ElastiCacheSigningError,
};
use betterlite_config::sections::redis_credentials::azure::{
    AZURE_REDIS_SCOPE, AzureAdCredentialProvider, AzureCredential, AzureSettings, AzureTokenFetcher,
};
use betterlite_config::sections::redis_credentials::gcp::GcpTokenFetcher;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_redis::redis_live_client::credentials::RedisCredentialError;
use secrecy::{ExposeSecret as _, SecretString};
use tokio::runtime::Handle;

use super::aws::SdkAwsCredentials;
use super::azure::{AzureGrantError, AzureGrants, AzureRedisTokens};
use super::azure_production::SdkAzureGrants;
use super::azure_runtime::{ProductionClock, ProductionHost, ThreadScheduler};
use super::gcp::{AdcTokenSource, IamTokenError, IamTokenFetcher, decode_iam_token};
use super::{
    AWS_REFRESH_AFTER, AZURE_REFRESH_AFTER, CloudRedisAuthDeps, GCP_REFRESH_AFTER, cloud_redis_auth,
};

mod support;

use support::{
    FakeIo, FixedAdc, GrantCall, RecordingGrants, Reply, aws_provider, aws_settings,
    azure_provider, azure_settings, deps, gcp_provider, iam_server, proxied_client,
    sourced_settings, static_aws_provider,
};

include!("tests/gcp.rs");
include!("tests/azure.rs");
include!("tests/aws.rs");
include!("tests/source.rs");
