use betterlite_a2a::client::A2AClient;
use betterlite_a2a::send::asend_message;
use betterlite_a2a::send::call::SendCall;
use betterlite_a2a::send::deps::BridgeDeps;
use pyo3::PyErr;

use super::convert::{Call, response_text};
use super::payload::entry_error;
use super::ports::{NoCompletion, NoProviders, SystemSource};
use super::setup::{Runtime, trace_ids};

// B4814's non-streaming entry: a client handle goes through as given, and api_base creates one when it is absent.
pub(crate) async fn send(client: Option<A2AClient>, call: Call) -> Result<String, PyErr> {
    let resolved = call.resolve(client)?;
    let wire = SendCall {
        a2a_client: resolved.client,
        request: resolved.request,
        api_base: resolved.api_base,
        litellm_params: resolved.litellm_params,
        agent_id: resolved.agent_id,
        agent_extra_headers: resolved.agent_extra_headers,
    };
    let runtime = Runtime::build()?;
    let trace = trace_ids(resolved.trace_id);
    let native = runtime.native(&trace);
    let completion = NoCompletion;
    let providers = NoProviders;
    let mut source = SystemSource;
    let mut bridge = BridgeDeps {
        completion: &completion,
        providers: &providers,
        source: &mut source,
    };
    let response = asend_message(wire, None, &native, &mut bridge)
        .await
        .map_err(|error| entry_error(error, false))?;
    Ok(response_text(&response)?)
}
