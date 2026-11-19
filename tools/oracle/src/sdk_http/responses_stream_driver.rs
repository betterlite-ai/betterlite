use std::collections::BTreeMap;
use std::future::Future;

use super::call_kind::CallKind;
use super::error::SdkHttpError;
use super::result::kinds::SdkResult;

mod assemble;
pub mod call;
mod run;
mod source;

pub(super) use call::{Call, Outcome};

impl CallKind for Call {
    fn send(
        &self,
        api_base: &str,
        env: &BTreeMap<String, String>,
    ) -> impl Future<Output = Result<SdkResult, SdkHttpError>> {
        send(self, api_base, env)
    }
}
async fn send(
    call: &Call,
    api_base: &str,
    env: &BTreeMap<String, String>,
) -> Result<SdkResult, SdkHttpError> {
    let client = super::vector_query_embedding::outbound_client(env)?;
    let mut runs = Vec::with_capacity(call.runs.len());
    for spec in &call.runs {
        runs.push(run::run(&client, call, spec, api_base).await?);
    }
    Ok(SdkResult::ResponsesStreamDriver {
        value: Box::new(runs),
    })
}
