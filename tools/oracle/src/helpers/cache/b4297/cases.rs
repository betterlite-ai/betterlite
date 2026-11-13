use std::sync::Arc;

use betterlite_cache::envelope::decode::decode_cached;
use betterlite_cache::envelope::facade::EnvelopeSettings;
use betterlite_cache::envelope::request::EnvelopeRequest;
use serde_json::json;

use super::output::{lookup_json, object_json, write_json};
use super::{
    FIXTURE_NOW, GetCacheInput, GetCacheLogicInput, HelperCache, RecordingBackend, WriteInput,
    cache, failure, rebuild, runtime,
};
use crate::helpers::HelperResult;

pub(crate) fn get_cache_logic(input: &GetCacheLogicInput) -> HelperResult {
    let result =
        decode_cached(input.cached.as_ref(), input.max_age, FIXTURE_NOW).map_err(failure)?;
    Ok(json!({"result": result}))
}

pub(crate) fn get_cache(input: &GetCacheInput) -> HelperResult {
    let backend = Arc::new(RecordingBackend::new(input.cached.clone(), input.stamp));
    let facade = cache(Arc::clone(&backend), &EnvelopeSettings::default(), None)?;
    let mut request = rebuild(&input.request);
    let result = facade.get_cache(None, &mut request);
    let snapshot = backend.snapshot();
    Ok(json!({
        "result": result,
        "metadata": request.key.metadata.as_ref().map(object_json),
        "lookup": snapshot.lookup.as_ref().map(lookup_json),
    }))
}

fn prepare(
    input: &WriteInput,
) -> Result<(Arc<RecordingBackend>, HelperCache, EnvelopeRequest), Box<crate::helpers::HelperError>>
{
    let backend = Arc::new(RecordingBackend::new(None, None));
    let facade = cache(Arc::clone(&backend), &input.settings, input.cache_type)?;
    let request = rebuild(&input.request);
    Ok((backend, facade, request))
}

pub(crate) fn add_cache(input: &WriteInput) -> HelperResult {
    let (backend, facade, mut request) = prepare(input)?;
    facade.add_cache(&mut request, &input.result);
    let snapshot = backend.snapshot();
    write_json(snapshot.write.as_ref())
}

pub(crate) fn async_add_cache(input: &WriteInput) -> HelperResult {
    let (backend, facade, mut request) = prepare(input)?;
    runtime()?.block_on(facade.async_add_cache(None, &mut request, &input.result));
    let snapshot = backend.snapshot();
    write_json(snapshot.write.as_ref())
}
