mod convert;
mod error;
mod native;
mod session;
mod tests;

use error::NativeAssistantError;
use native::NativeAssistantSession;
use pyo3::prelude::*;

// litellm/assistants/main.py's ten functions: one session per call and the exception every failure carries.
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<NativeAssistantSession>()?;
    module.add(
        "NativeAssistantError",
        module.py().get_type::<NativeAssistantError>(),
    )?;
    Ok(())
}
