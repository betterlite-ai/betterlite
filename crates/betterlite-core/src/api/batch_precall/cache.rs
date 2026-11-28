use betterlite_cache::dual_cache::cache_from_json;
use betterlite_cache::envelope::decode::decode_cached;
use betterlite_cache::handler::value::to_json;
use betterlite_models::decisions::Json;
use betterlite_redact::structured::REDACTED;

use crate::api::api_boundaries::{ApiBoundariesInput, ApiLimits, JsonBoundary};
use crate::api::batch_input_accounting::error::BatchHttpError;
use crate::api::batch_limit_counters::records::EpochClock;
use crate::api::request_destination::RequestData;

use super::BatchPrecallError;
use super::fields;
use super::input::BatchRetrieveCall;
use super::ports::{BatchCacheError, BatchPrecallMemory, BatchPrecallRemote};
use super::result::{BatchDiagnostic, BatchRetrieveResult, debug, error};

const CACHE_FAILURE_PREFIX: &str = concat!(
    "litellm.proxy.hooks.batch_redis_get.py::async_pre_call_hook(): Exception occu",
    "red - "
);
const SCAN_COUNT: usize = 100;
const WARM_TTL_SECONDS: f64 = 60.0;

pub(crate) struct RetrieveCache<'cache, Memory, Remote> {
    pub(crate) memory: &'cache Memory,
    pub(crate) remote: Option<&'cache Remote>,
    pub(crate) verbose_stdout: bool,
    pub(crate) clock: &'cache EpochClock,
    pub(crate) limits: ApiLimits,
}

// the verbose sink a retrieve writes through, its lines already masked
struct Verbose {
    diagnostics: Vec<BatchDiagnostic>,
    printed: Vec<String>,
}

impl<Memory: BatchPrecallMemory, Remote: BatchPrecallRemote> RetrieveCache<'_, Memory, Remote> {
    pub(crate) async fn run(
        &self,
        call: &BatchRetrieveCall,
    ) -> Result<BatchRetrieveResult, BatchPrecallError> {
        let mut data = call.data.clone();
        let api_key = fields::principal_text(&call.principal, "api_key")?;
        let name = format!("litellm:{}:{}", api_key.unwrap_or("None"), call.call_type);
        // secrets-stay-secret: the verbose lines mask the key inside the name, while the namespace and cache key keep it raw
        let masked_name = match api_key {
            Some(_) => format!("litellm:{REDACTED}:{}", call.call_type),
            None => name.clone(),
        };
        let mut verbose = Verbose {
            diagnostics: Vec::new(),
            printed: Vec::new(),
        };
        let raised = self
            .warm(&name, &masked_name, &mut data, &mut verbose)
            .await;
        let cached = if raised.is_some() {
            None
        } else {
            self.read(call, &data).await
        };
        Ok(BatchRetrieveResult {
            data,
            cached,
            raised,
            diagnostics: verbose.diagnostics,
            printed: verbose.printed,
        })
    }

    // async_pre_call_hook: with no in-memory key for the namespace, scan redis once, pipeline-read the batch, and warm memory for a minute
    async fn warm(
        &self,
        name: &str,
        masked_name: &str,
        data: &mut RequestData,
        verbose: &mut Verbose,
    ) -> Option<BatchHttpError> {
        let mut warmed = Vec::new();
        if !self.memory.keys().iter().any(|key| key.starts_with(name))
            && let Some(remote) = self.remote
            && remote.is_redis()
        {
            self.print_verbose(&format!("cache_key_name: {masked_name}"), verbose);
            let keys = match remote.scan(name, SCAN_COUNT).await {
                Ok(keys) => keys,
                Err(failure) => return warm_failed(failure, &mut verbose.diagnostics),
            };
            let listed: Vec<String> = keys
                .iter()
                .map(|key| {
                    let suffix = key.strip_prefix(name).unwrap_or_default();
                    format!("'{masked_name}{suffix}'")
                })
                .collect();
            self.print_verbose(&format!("redis keys: [{}]", listed.join(", ")), verbose);
            if !keys.is_empty() {
                match remote.batch_get(&keys).await {
                    Ok(entries) => warmed = entries,
                    Err(failure) => return warm_failed(failure, &mut verbose.diagnostics),
                }
            }
        }
        if !warmed.is_empty()
            && let Err(failure) = self.memory.set_many(warmed, WARM_TTL_SECONDS).await
        {
            return warm_failed(failure, &mut verbose.diagnostics);
        }
        if let Err(reason) = fields::set_namespace(data, name) {
            let message = format!("{CACHE_FAILURE_PREFIX}{reason}");
            verbose.diagnostics.push(error(message));
        }
        None
    }

    // print_verbose: the debug log line always, and the same text on stdout when litellm.set_verbose is True
    fn print_verbose(&self, line: &str, verbose: &mut Verbose) {
        let message = line.to_owned();
        verbose.diagnostics.push(debug(message.clone()));
        if self.verbose_stdout {
            verbose.printed.push(message);
        }
    }

    // async_get_cache: memory first, the backing store on a miss with the hit warmed back, and every failure answered with None
    async fn read(&self, call: &BatchRetrieveCall, data: &RequestData) -> Option<Json> {
        let remote = self.remote?;
        let key = match &call.cache_key {
            Some(key) => key.clone(),
            None => remote.cache_key(data)?,
        };
        let cached = match self.memory.get(&key) {
            Ok(Some(value)) => value,
            Ok(None) => {
                let value = remote.get(&key).await.ok()??;
                self.memory
                    .set(&key, value.clone(), WARM_TTL_SECONDS)
                    .await
                    .ok()?;
                value
            }
            Err(_failure) => return None,
        };
        decode(
            &cached,
            call.cache_control.and_then(|control| control.max_age),
            self.clock,
            self.limits,
        )
    }
}

fn warm_failed(
    failure: BatchCacheError,
    diagnostics: &mut Vec<BatchDiagnostic>,
) -> Option<BatchHttpError> {
    match failure {
        BatchCacheError::Http { status, detail } => Some(BatchHttpError { status, detail }),
        BatchCacheError::Failed { message } => {
            diagnostics.push(error(format!("{CACHE_FAILURE_PREFIX}{message}")));
            None
        }
    }
}

// litellm.cache._get_cache_logic behind the typed cache value model, bounded by the JSON limit like every outside payload
fn decode(
    cached: &Json,
    max_age: Option<i64>,
    clock: &EpochClock,
    limits: ApiLimits,
) -> Option<Json> {
    let bytes = serde_json::to_vec(cached).ok()?;
    let boundary = ApiBoundariesInput::Json(JsonBoundary {
        known_bytes: bytes.len(),
    });
    limits.check(boundary).ok()?;
    let value = cache_from_json(cached).ok()?;
    let decoded = decode_cached(Some(&value), max_age, (clock)()).ok()??;
    to_json(&decoded)
}
