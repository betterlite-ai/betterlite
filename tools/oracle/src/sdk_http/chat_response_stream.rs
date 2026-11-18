use std::collections::BTreeMap;

use super::error::SdkHttpError;
use super::result::kinds::SdkResult;
use super::wrapper_stream;
use call::ChatResponseStreamCall;

mod assembly;
pub mod call;
mod run;
mod source;

pub(crate) async fn send(
    call: &ChatResponseStreamCall,
    api_base: &str,
    env: &BTreeMap<String, String>,
) -> Result<SdkResult, SdkHttpError> {
    let prepared = wrapper_stream::prepare(call, api_base, env)?;
    // every run gets its own POST, all of them sent before the first run drives its payloads
    let mut opened = Vec::with_capacity(call.runs.len());
    for _ in &call.runs {
        opened.push(run::open(&prepared, call).await?);
    }
    let mut runs = Vec::with_capacity(opened.len());
    for (spec, answer) in call.runs.iter().zip(opened) {
        runs.push(run::run(call, spec, answer).await?);
    }
    Ok(SdkResult::ChatResponseStream { value: runs })
}
