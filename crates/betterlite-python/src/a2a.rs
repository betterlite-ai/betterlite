mod api;
mod card;
mod client;
mod convert;
mod error;
mod payload;
mod ports;
mod send;
mod setup;
mod stream;

use self::api::{
    create_client, fix_agent_card_url, get_agent_card, get_agent_card_url,
    is_localhost_or_internal_url, normalized_card, send_message_direct,
    send_message_streaming_direct, set_agent_card_url,
};
use self::client::NativeA2aClient;
use self::error::NativeA2aError;
use self::stream::NativeA2aStream;
use pyo3::prelude::*;

// The names litellm.a2a_protocol reaches: one client, its streams, the card helpers and the error every failure carries.
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<NativeA2aClient>()?;
    module.add_class::<NativeA2aStream>()?;
    module.add("NativeA2aError", module.py().get_type::<NativeA2aError>())?;
    module.add_function(wrap_pyfunction!(create_client, module)?)?;
    module.add_function(wrap_pyfunction!(get_agent_card, module)?)?;
    module.add_function(wrap_pyfunction!(send_message_direct, module)?)?;
    module.add_function(wrap_pyfunction!(send_message_streaming_direct, module)?)?;
    module.add_function(wrap_pyfunction!(is_localhost_or_internal_url, module)?)?;
    module.add_function(wrap_pyfunction!(normalized_card, module)?)?;
    module.add_function(wrap_pyfunction!(get_agent_card_url, module)?)?;
    module.add_function(wrap_pyfunction!(set_agent_card_url, module)?)?;
    module.add_function(wrap_pyfunction!(fix_agent_card_url, module)?)?;
    Ok(())
}
