mod convert;
mod error;
mod helpers;
mod native;
mod session;
mod stream;

use error::NativePassthroughError;
use helpers::{
    encode_arn, forward_headers_from_request, merge_query_parameters, resolve_passthrough_timeout,
    resolve_sdk_timeout,
};
use pyo3::prelude::*;

// One session, its streams, the exception every failure carries and the passthrough helpers.
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<native::NativePassthroughSession>()?;
    module.add_class::<stream::NativePassthroughStream>()?;
    module.add(
        "NativePassthroughError",
        module.py().get_type::<NativePassthroughError>(),
    )?;
    module.add_function(wrap_pyfunction!(merge_query_parameters, module)?)?;
    module.add_function(wrap_pyfunction!(forward_headers_from_request, module)?)?;
    module.add_function(wrap_pyfunction!(encode_arn, module)?)?;
    module.add_function(wrap_pyfunction!(resolve_passthrough_timeout, module)?)?;
    module.add_function(wrap_pyfunction!(resolve_sdk_timeout, module)?)?;
    Ok(())
}
