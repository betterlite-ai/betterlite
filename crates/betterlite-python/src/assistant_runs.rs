mod convert;
mod error;
mod native;
mod session;
mod stream;
mod tests;

use error::NativeAssistantRunError;
use native::NativeAssistantRunSession;
use pyo3::prelude::*;
use stream::NativeAssistantStream;

// litellm/assistants/main.py's message and run calls: one session per call, its exception and the event stream.
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<NativeAssistantRunSession>()?;
    module.add_class::<NativeAssistantStream>()?;
    module.add(
        "NativeAssistantRunError",
        module.py().get_type::<NativeAssistantRunError>(),
    )?;
    Ok(())
}
