#![cfg(test)]

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use aws_credential_types::credential_fn::provide_credentials_fn;
use aws_credential_types::provider::SharedCredentialsProvider;
use aws_credential_types::provider::error::CredentialsError;
use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode, Uri};
use axum::response::{IntoResponse as _, Response};
use axum::routing::any;
use betterlite_azure::azure_arc_identity::{ArcHost, ArcPlatform, ArcTransport, MetadataRequest};
use betterlite_azure::azure_certificate::assertion::RandomIds;
use betterlite_azure::azure_certificate::credential::StdFiles;
use betterlite_azure::azure_client_secret::transport::{
    Clock as TokenClock, HttpMethod, HttpRequest, Transport as ClientSecretTransport,
};
use betterlite_azure::azure_default_chain::environment::ChainEnvironment;
use betterlite_azure::azure_default_chain::grant::ChainToken;
use betterlite_azure::azure_hosted_identity::request::{HostedRequest, HostedTransport};
use betterlite_azure::azure_managed_identity::environment::ManagedIdentityEnvironment;
use betterlite_azure::azure_managed_identity::imds::ImdsTransport;
use betterlite_azure::azure_managed_identity::runtime::{FsTokenFiles, Scheduler};
use betterlite_azure::azure_username_password::transport::{
    HttpRequest as PasswordRequest, Transport as PasswordTransport,
};
use betterlite_azure::oidc_exchange::AzureOidcExchange;
use betterlite_azure::oidc_exchange::transport::{Clock as OidcClock, Transport as OidcTransport};
use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::azure_client_secret::Settings as ClientSecretSettings;
use betterlite_config::sections::azure_oidc_exchange::Settings as OidcSettings;
use betterlite_config::sections::redis_credentials::Settings;
use betterlite_config::sections::redis_credentials::aws::{
    AwsCredentialsResolver, ElastiCacheIamProvider, ElastiCacheSigningError,
};
use betterlite_config::sections::redis_credentials::azure::{
    AZURE_REDIS_SCOPE, AzureAdCredentialProvider, AzureCredential, AzureSettings,
    AzureTokenFetcher as _,
};
use betterlite_config::sections::redis_credentials::gcp::GcpTokenFetcher;
use betterlite_config::sections::vertex_credential_sources::vertex_settings::AdcEnvironment;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_redis::redis_live_client::credentials::RedisCredentialError;
use betterlite_vertex::credential_sources::files::StdFiles as VertexStdFiles;
use betterlite_vertex::credential_sources::sdk::{GoogleAuth, HttpPolicy};
use secrecy::{ExposeSecret as _, SecretString};
use tokio::net::TcpListener;
use tokio::runtime::Handle;
use tokio::task::JoinSet;

use super::aws::SdkAwsCredentials;
use super::azure::{AzureGrantError, AzureGrants, AzureRedisTokens};
use super::azure_http::AzureHttp;
use super::azure_production::{KeptProvider, SdkAzureGrants, TokenCache};
use super::azure_runtime::{ProductionClock, ProductionHost, ThreadScheduler};
use super::gcp::{AdcError, AdcTokenSource, IamTokenError, IamTokenFetcher, decode_iam_token};
use super::gcp_production::VertexAdcTokenSource;
use super::production::production_cloud_redis_auth;
use super::{
    AWS_REFRESH_AFTER, AZURE_REFRESH_AFTER, CloudRedisAuthDeps, GCP_REFRESH_AFTER, cloud_redis_auth,
};

mod support;

use support::{
    FakeIo, FixedAdc, GrantCall, RecordingGrants, Reply, aws_provider, aws_settings,
    azure_provider, azure_settings, deps, gcp_provider, iam_server, proxied_client, run_blocking,
    sourced_settings, static_aws_provider,
};

include!("tests/gcp.rs");
include!("tests/azure.rs");
include!("tests/azure_http.rs");
include!("tests/azure_runtime.rs");
include!("tests/aws.rs");
include!("tests/production.rs");
include!("tests/source.rs");
