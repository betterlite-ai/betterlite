mod convert;
mod error;
mod payload;
mod session;
mod tests;
mod transport;

use self::error::NativeBatchListError;
use self::session::NativeBatchListSession;
use pyo3::prelude::*;

// The batch listing classes litellm.batches reaches for: one session and the exception every failure carries.
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<NativeBatchListSession>()?;
    module.add(
        "NativeBatchListError",
        module.py().get_type::<NativeBatchListError>(),
    )?;
    Ok(())
}
