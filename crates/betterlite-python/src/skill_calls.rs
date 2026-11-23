mod convert;
mod error;
mod hosted;
mod native;
mod session;
mod transport;
use self::error::NativeSkillError;
use self::native::NativeSkillSession;
use pyo3::prelude::*;
// The classes betterlite.skills reaches for: one session per call and the exception every failure carries.
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<NativeSkillSession>()?;
    module.add(
        "NativeSkillError",
        module.py().get_type::<NativeSkillError>(),
    )?;
    Ok(())
}
