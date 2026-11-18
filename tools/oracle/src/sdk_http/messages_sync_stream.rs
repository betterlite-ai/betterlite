use std::collections::BTreeMap;

use super::error::SdkHttpError;
use super::result::SyncRun;
use super::result::kinds::SdkResult;
use super::wrapper_stream;
use call::MessagesSyncStreamCall;

pub mod call;
pub(crate) mod diagnostics;
mod run;
mod source;

pub(crate) async fn send(
    call: &MessagesSyncStreamCall,
    api_base: &str,
    env: &BTreeMap<String, String>,
) -> Result<SdkResult, SdkHttpError> {
    let prepared = wrapper_stream::prepare(call, api_base, env)?;
    let mut runs = Vec::with_capacity(call.runs.len());
    for spec in &call.runs {
        let payloads = wrapper_stream::prefetch(&prepared, call).await?;
        runs.push(run::run(call, spec, payloads)?);
    }
    Ok(SdkResult::MessagesSyncStream {
        value: SyncRun { runs },
    })
}
