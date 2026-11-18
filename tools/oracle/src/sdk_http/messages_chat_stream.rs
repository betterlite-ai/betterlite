use std::collections::BTreeMap;

use super::error::SdkHttpError;
use super::result::ChatStreamRun;
use super::result::kinds::SdkResult;
use super::wrapper_stream;
use call::MessagesChatStreamCall;

pub mod call;
mod run;
mod source;

pub(crate) async fn send(
    call: &MessagesChatStreamCall,
    api_base: &str,
    env: &BTreeMap<String, String>,
) -> Result<SdkResult, SdkHttpError> {
    let prepared = wrapper_stream::prepare(call, api_base, env)?;
    // every run gets its own POST, all of them sent before the first run drives its payloads
    let mut prefetched = Vec::with_capacity(call.runs.len());
    for _ in &call.runs {
        prefetched.push(wrapper_stream::prefetch(&prepared, call).await?);
    }
    let mut runs = Vec::with_capacity(prefetched.len());
    for (spec, payloads) in call.runs.iter().zip(prefetched) {
        runs.push(run::run(call, spec, payloads).await?);
    }
    Ok(SdkResult::MessagesChatStream {
        value: ChatStreamRun { runs },
    })
}
