use pyo3::prelude::*;

use self::disk::NativeDisk;
use self::dual::NativeDual;
use self::dual::batch::{NativeDeclaredBatchRead, NativeDeferred, NativeRedisBatch};
use self::error::RedisCircuitBreakerOpenError;
use self::memory::NativeMemory;
use self::redis::NativeRedis;

mod blocking;
mod convert;
mod disk;
mod dual;
mod error;
mod memory;
mod redis;
mod settings;

pub fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add(
        "RedisCircuitBreakerOpenError",
        module.py().get_type::<RedisCircuitBreakerOpenError>(),
    )?;
    module.add_class::<NativeMemory>()?;
    module.add_class::<NativeDisk>()?;
    module.add_class::<NativeRedis>()?;
    module.add_class::<NativeDual>()?;
    module.add_class::<NativeDeferred>()?;
    module.add_class::<NativeDeclaredBatchRead>()?;
    module.add_class::<NativeRedisBatch>()?;
    Ok(())
}

mod tests;
