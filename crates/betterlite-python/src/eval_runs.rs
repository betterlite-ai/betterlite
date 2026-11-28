mod configs;
mod convert;
mod error;
mod session;
mod transport;

use pyo3::prelude::*;

use self::error::NativeEvalRunError;
use self::session::NativeEvalRunSession;

// _runs.py's run calls: one session owning the pooled client, its settings and the provider config.
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<NativeEvalRunSession>()?;
    module.add(
        "NativeEvalRunError",
        module.py().get_type::<NativeEvalRunError>(),
    )?;
    Ok(())
}
