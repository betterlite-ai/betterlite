use std::io::Write;
use std::path::Path;
use std::sync::Arc;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_logging::private_json::files::PrivateFiles;
use clap::ArgMatches;
use secrecy::SecretString;
use tokio::runtime::Builder;

use crate::cli_context::CliContext;
use crate::commands::Client;
use crate::config::{load_config, normalize_base_url};
use crate::credential_lifecycle::files::ManagedTokenFile;
use crate::credentials::notice::MessageSink;
use crate::credentials::storage::{StoredApiKeyRequest, get_stored_api_key};
use crate::keyring::SecretVault;

use super::Failure;
use super::root::{API_KEY_ARG, BASE_URL_ARG};
use super::session::CliClock;
use super::streams::Channel;

pub(crate) const DEFAULT_BASE_URL: &str = "http://localhost:4000";
pub(crate) const BASE_URL_ENV: &str = "LITELLM_PROXY_URL";
pub(crate) const API_KEY_ENV: &str = "LITELLM_PROXY_API_KEY";
const SETUP_COMMANDS: [&str; 3] = ["configure", "reconfigure", "unconfigure"];

pub(crate) struct GlobalArgs {
    pub base_url: Option<String>,
    pub api_key: Option<SecretString>,
}

pub(crate) struct Request<'request> {
    pub environment: &'request (dyn EnvironmentLookup + Send + Sync),
    pub globals: GlobalArgs,
    pub subcommand: Option<&'request str>,
    pub vault: &'request mut (dyn SecretVault + Send + Sync),
    pub clock: Arc<CliClock>,
    pub token_path: Option<&'request Path>,
    pub stderr: Channel,
}

struct StoredKey<'stored> {
    client: Arc<Client>,
    base_url: &'stored str,
    clock: Arc<CliClock>,
    vault: &'stored mut (dyn SecretVault + Send + Sync),
    token_path: &'stored Path,
    warn: MessageSink<'stored>,
}

#[must_use]
pub(crate) fn globals(matches: &ArgMatches) -> GlobalArgs {
    GlobalArgs {
        base_url: matches.get_one::<String>(BASE_URL_ARG).cloned(),
        api_key: matches
            .get_one::<String>(API_KEY_ARG)
            .map(SecretString::from),
    }
}

pub(crate) fn resolve_for_run(mut request: Request<'_>) -> Result<CliContext, Failure> {
    let setup = request
        .subcommand
        .is_some_and(|name| SETUP_COMMANDS.contains(&name));
    let (chosen, stored, provided) =
        base_url_choice(request.environment, &request.globals, &mut request.stderr);
    let explicit_api_key = request.globals.api_key.take().or_else(|| {
        request
            .environment
            .lookup(API_KEY_ENV)
            .filter(|value| !value.is_empty())
            .map(SecretString::from)
    });
    let from_token_file = explicit_api_key.is_none() && !setup;
    let context = CliContext {
        base_url: normalize_base_url(&chosen).map_err(Failure::from)?,
        base_url_explicit: provided.is_some() || (stored.is_some() && !setup),
        api_key: explicit_api_key,
        api_key_from_token_file: from_token_file,
    };
    if !from_token_file {
        return Ok(context);
    }
    with_stored_key(context, &mut request)
}

fn base_url_choice(
    environment: &dyn EnvironmentLookup,
    globals: &GlobalArgs,
    warnings: &mut dyn Write,
) -> (String, Option<String>, Option<String>) {
    let stored = load_config(environment, warnings)
        .get("base_url")
        .cloned()
        .filter(|value| !value.is_empty());
    let provided = globals.base_url.clone().or_else(|| {
        environment
            .lookup(BASE_URL_ENV)
            .filter(|value| !value.is_empty())
    });
    let chosen = provided
        .clone()
        .or_else(|| stored.clone())
        .unwrap_or_else(|| DEFAULT_BASE_URL.to_owned());
    (chosen, stored, provided)
}

fn with_stored_key(
    mut context: CliContext,
    request: &mut Request<'_>,
) -> Result<CliContext, Failure> {
    let Some(token_path) = request.token_path else {
        return Ok(context);
    };
    let stderr = request.stderr.clone();
    let warn = move |message: &str| {
        let mut sink = stderr.clone();
        let _unused = writeln!(sink, "{message}");
    };
    let client = Arc::new(Client::new(&context, request.environment).map_err(Failure::from)?);
    context.api_key = stored_api_key(StoredKey {
        client,
        base_url: &context.base_url,
        clock: Arc::clone(&request.clock),
        vault: &mut *request.vault,
        token_path,
        warn: &warn,
    })?;
    Ok(context)
}

fn stored_api_key(request: StoredKey<'_>) -> Result<Option<SecretString>, Failure> {
    let files = ManagedTokenFile {
        files: PrivateFiles::default(),
        path: request.token_path.to_owned(),
    };
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|source| Failure::Plain(format!("the async runtime could not start: {source}")))?;
    Ok(runtime.block_on(get_stored_api_key(StoredApiKeyRequest {
        expected_base_url: Some(request.base_url),
        files: &files,
        vault: request.vault,
        http: request.client,
        clock: request.clock,
        warn: request.warn,
        token_path: request.token_path,
    })))
}
