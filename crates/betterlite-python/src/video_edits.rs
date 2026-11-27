mod adapter;
mod convert;
mod error;
mod native;
mod operations;
mod session;

use self::error::NativeVideoEditError;
use self::native::{NativeVideoEditSession, is_litellm_owned_kwarg};
use pyo3::prelude::*;
use pyo3::wrap_pyfunction;

// characters, edits and extensions: one native session, the call error and the keyword-name predicate
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<NativeVideoEditSession>()?;
    let error = module.py().get_type::<NativeVideoEditError>();
    module.add("NativeVideoEditError", error)?;
    module.add_function(wrap_pyfunction!(is_litellm_owned_kwarg, module)?)
}
