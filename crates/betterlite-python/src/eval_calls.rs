mod configs;
mod convert;
mod error;
mod payload;
mod session;

use pyo3::prelude::*;

use self::error::NativeEvalError;
use self::session::NativeEvalSession;

// main.py's eval calls: one session owning the pooled client, its settings and the provider config.
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<NativeEvalSession>()?;
    module.add("NativeEvalError", module.py().get_type::<NativeEvalError>())?;
    Ok(())
}
