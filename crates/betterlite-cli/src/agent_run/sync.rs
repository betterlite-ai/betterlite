use super::codex::{
    DebugModels, DebugModelsOutcome, StockModels, codex_debug_models, codex_model_catalog_path,
    replace_file, stock_codex_models,
};
use super::error::AgentRunError;
use crate::agent_launch::EnvVars;
use crate::codex_catalog::codex_model_catalog;
use crate::opencode::{
    ModelListingTransport, ModelSyncArgs, ModelSyncResult, ModelSyncSkipped, fetch_model_listing,
    opencode_model_sync_env,
};
use betterlite_logging::private_json::files::FileAccess;
use secrecy::SecretString;
use std::ffi::OsStr;
use std::io;
use std::path::Path;
pub const SKIP_VERIFY_FLAG: &str = "--skip-verify";
pub struct SyncRequest<'request> {
    pub binary: &'request str,
    pub base_env: &'request EnvVars,
    pub base_url: &'request str,
    pub api_key: &'request SecretString,
    pub skip_verify: bool,
}
pub struct SyncContext<'context> {
    pub get: &'context dyn ModelListingTransport,
    pub debug: &'context dyn DebugModels,
    pub files: &'context dyn FileAccess,
    pub home: Option<&'context Path>,
    pub instructions: &'context str,
}
pub struct CodexSyncRequest<'request> {
    pub base_env: &'request EnvVars,
    pub base_url: &'request str,
    pub api_key: &'request SecretString,
    pub binary: &'request str,
}
pub trait ModelSync {
    fn sync(&self, request: &SyncRequest<'_>) -> Result<ModelSyncResult, AgentRunError>;
}
pub struct AgentModelSync<'context> {
    pub context: SyncContext<'context>,
}
impl ModelSync for AgentModelSync<'_> {
    fn sync(&self, request: &SyncRequest<'_>) -> Result<ModelSyncResult, AgentRunError> {
        agent_model_sync_env(request, &self.context)
    }
}
pub fn agent_model_sync_env(
    request: &SyncRequest<'_>,
    context: &SyncContext<'_>,
) -> Result<ModelSyncResult, AgentRunError> {
    let stem = Path::new(request.binary)
        .file_stem()
        .and_then(OsStr::to_str)
        .filter(|name| matches!(*name, "codex" | "opencode"));
    let Some(agent) = stem else {
        return Ok(ModelSyncResult::Env(EnvVars::new()));
    };
    if request.skip_verify {
        return Ok(ModelSyncResult::Skipped(ModelSyncSkipped {
            reason: format!("{SKIP_VERIFY_FLAG} was passed"),
        }));
    }
    if agent == "codex" {
        let codex_request = CodexSyncRequest {
            base_env: request.base_env,
            base_url: request.base_url,
            api_key: request.api_key,
            binary: request.binary,
        };
        return codex_model_sync_args(&codex_request, context);
    }
    Ok(opencode_model_sync_env(
        request.base_env,
        request.base_url,
        request.api_key,
        context.get,
    ))
}
pub fn codex_model_sync_args(
    request: &CodexSyncRequest<'_>,
    context: &SyncContext<'_>,
) -> Result<ModelSyncResult, AgentRunError> {
    let listing = match fetch_model_listing(request.base_url, request.api_key, context.get) {
        Ok(listing) => listing,
        Err(skipped) => return Ok(ModelSyncResult::Skipped(skipped)),
    };
    let Some(path) = codex_model_catalog_path(request.base_env, context.home) else {
        return Ok(ModelSyncResult::Skipped(ModelSyncSkipped {
            reason: "could not locate a home directory for the Codex model catalog; set CODEX_HOME"
                .to_owned(),
        }));
    };
    if let Err(source) = create_catalog_parent(&path, context.files) {
        return Ok(ModelSyncResult::Skipped(ModelSyncSkipped {
            reason: format!("could not write {}: {source}", path.display()),
        }));
    }
    let stock = match stock_codex_models(request.binary, request.base_env, context.debug)? {
        StockModels::Models(models) => models,
        StockModels::Skipped(skipped) => return Ok(ModelSyncResult::Skipped(skipped)),
    };
    let Some(catalog) = codex_model_catalog(&listing, &stock, context.instructions) else {
        return Ok(ModelSyncResult::Skipped(no_chat_models_skip(
            request.base_url,
        )));
    };
    if let Err(source) = replace_file(&path, &catalog, context.files) {
        return Ok(ModelSyncResult::Skipped(ModelSyncSkipped {
            reason: format!("could not write {}: {source}", path.display()),
        }));
    }
    read_back_catalog(request, context, &path)
}
fn read_back_catalog(
    request: &CodexSyncRequest<'_>,
    context: &SyncContext<'_>,
    path: &Path,
) -> Result<ModelSyncResult, AgentRunError> {
    let Some(path_text) = path.to_str() else {
        return Ok(ModelSyncResult::Skipped(ModelSyncSkipped {
            reason: format!(
                "the Codex model catalog path {} is not valid UTF-8",
                path.display()
            ),
        }));
    };
    let override_arg = format!("model_catalog_json={}", serde_json::json!(path_text));
    let read_back_args = ["-c".to_owned(), override_arg];
    match codex_debug_models(
        request.binary,
        &read_back_args,
        request.base_env,
        context.debug,
    )? {
        DebugModelsOutcome::Printed(_printed) => Ok(ModelSyncResult::Args(ModelSyncArgs {
            args: read_back_args.to_vec(),
        })),
        DebugModelsOutcome::Skipped(skipped) => Ok(ModelSyncResult::Skipped(skipped)),
    }
}
fn create_catalog_parent(path: &Path, files: &dyn FileAccess) -> io::Result<()> {
    let directory = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    files.create_parents(directory)
}
fn no_chat_models_skip(base_url: &str) -> ModelSyncSkipped {
    let root = base_url.trim_end_matches('/');
    ModelSyncSkipped {
        reason: format!("{root}/v1/models lists no chat models"),
    }
}
