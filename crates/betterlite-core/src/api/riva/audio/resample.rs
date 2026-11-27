use pyo3::exceptions::PyImportError;
use pyo3::types::{PyAnyMethods as _, PyDict, PyDictMethods as _, PyInt, PyModule};
use pyo3::{Bound, PyAny, PyErr, PyResult, Python};

use super::TARGET_SAMPLE_RATE_HZ;
use super::error::{RivaAudioError, call_failure};
use super::python::{as_int, import_numpy};

pub(super) fn resample<'py>(
    py: Python<'py>,
    samples: &Bound<'py, PyAny>,
    source_rate: i64,
) -> Result<Bound<'py, PyAny>, RivaAudioError> {
    let numpy = import_numpy(py)?;
    let target_rate = i64::from(TARGET_SAMPLE_RATE_HZ);
    let size = samples
        .getattr("size")
        .and_then(|size| as_int(py, &size))
        .map_err(|error| call_failure(py, "samples.size", &error))?;
    if source_rate == target_rate || size == 0 {
        return Ok(samples.clone());
    }
    for resampler in [Resampler::Soxr, Resampler::Scipy] {
        if let Some(converted) =
            try_resampler(py, &numpy, resampler, samples, source_rate, target_rate)?
        {
            return Ok(converted);
        }
    }
    linear(py, &numpy, samples, source_rate, target_rate)
}

#[derive(Clone, Copy)]
enum Resampler {
    Soxr,
    Scipy,
}

fn try_resampler<'py>(
    py: Python<'py>,
    numpy: &Bound<'py, PyModule>,
    resampler: Resampler,
    samples: &Bound<'py, PyAny>,
    source_rate: i64,
    target_rate: i64,
) -> Result<Option<Bound<'py, PyAny>>, RivaAudioError> {
    let outcome = match resampler {
        Resampler::Soxr => py
            .import("soxr")
            .and_then(|soxr| soxr.getattr("resample"))
            .and_then(|call| call.call1((samples, source_rate, target_rate)))
            .and_then(|converted| as_float32(py, numpy, &converted)),
        Resampler::Scipy => scipy_resample(py, samples, source_rate, target_rate)
            .and_then(|converted| as_float32(py, numpy, &converted)),
    };
    match outcome {
        Ok(converted) => Ok(Some(converted)),
        Err(error) if error.is_instance_of::<PyImportError>(py) => Ok(None),
        Err(error) => Err(call_failure(py, operation(resampler), &error)),
    }
}

fn scipy_resample<'py>(
    py: Python<'py>,
    samples: &Bound<'py, PyAny>,
    source_rate: i64,
    target_rate: i64,
) -> PyResult<Bound<'py, PyAny>> {
    let target_int = PyInt::new(py, target_rate);
    let source_int = PyInt::new(py, source_rate);
    let divisor = py
        .import("math")?
        .getattr("gcd")?
        .call1((&source_int, &target_int))?;
    let up = target_int.call_method1("__floordiv__", (&divisor,))?;
    let down = source_int.call_method1("__floordiv__", (&divisor,))?;
    py.import("scipy.signal")?
        .getattr("resample_poly")?
        .call1((samples, up, down))
}

const fn operation(resampler: Resampler) -> &'static str {
    match resampler {
        Resampler::Soxr => "soxr.resample",
        Resampler::Scipy => "scipy.signal.resample_poly",
    }
}

fn as_float32<'py>(
    py: Python<'py>,
    numpy: &Bound<'py, PyModule>,
    value: &Bound<'py, PyAny>,
) -> PyResult<Bound<'py, PyAny>> {
    let keywords = PyDict::new(py);
    keywords.set_item("dtype", numpy.getattr("float32")?)?;
    numpy.getattr("asarray")?.call((value,), Some(&keywords))
}

fn linear<'py>(
    py: Python<'py>,
    numpy: &Bound<'py, PyModule>,
    samples: &Bound<'py, PyAny>,
    source_rate: i64,
    target_rate: i64,
) -> Result<Bound<'py, PyAny>, RivaAudioError> {
    let fail = |error: PyErr| call_failure(py, "linear resample", &error);
    let size = samples.getattr("size").map_err(fail)?;
    let target_length = linear_target_length(py, &size, source_rate, target_rate).map_err(fail)?;
    let float32 = numpy.getattr("float32").map_err(fail)?;
    if target_length <= 1 {
        return samples.call_method1("astype", (&float32,)).map_err(fail);
    }
    interpolate(py, numpy, samples, &size, target_length).map_err(fail)
}

fn linear_target_length(
    py: Python<'_>,
    size: &Bound<'_, PyAny>,
    source_rate: i64,
    target_rate: i64,
) -> PyResult<i64> {
    let builtins = py.import("builtins")?;
    let rate = builtins
        .getattr("float")?
        .call1((PyInt::new(py, source_rate),))?;
    let duration = size.call_method1("__truediv__", (&rate,))?;
    let product = duration.call_method1("__mul__", (PyInt::new(py, target_rate),))?;
    let rounded = builtins.getattr("round")?.call1((&product,))?;
    as_int(py, &rounded)
}

fn interpolate<'py>(
    py: Python<'py>,
    numpy: &Bound<'py, PyModule>,
    samples: &Bound<'py, PyAny>,
    size: &Bound<'py, PyAny>,
    target_length: i64,
) -> PyResult<Bound<'py, PyAny>> {
    let float32 = numpy.getattr("float32")?;
    let last = size.call_method1("__sub__", (1_i64,))?;
    let keywords = PyDict::new(py);
    keywords.set_item("num", target_length)?;
    keywords.set_item("dtype", numpy.getattr("float64")?)?;
    let indices = numpy
        .getattr("linspace")?
        .call((0_i64, &last), Some(&keywords))?;
    let int64 = numpy.getattr("int64")?;
    let left = numpy
        .getattr("floor")?
        .call1((&indices,))?
        .call_method1("astype", (&int64,))?;
    let right = numpy
        .getattr("minimum")?
        .call1((left.call_method1("__add__", (1_i64,))?, &last))?;
    let fraction = indices
        .call_method1("__sub__", (&left,))?
        .call_method1("astype", (&float32,))?;
    let complement = fraction.call_method1("__rsub__", (1.0_f64,))?;
    let left_samples = samples.get_item(&left)?;
    let right_samples = samples.get_item(&right)?;
    let second = fraction.call_method1("__mul__", (&right_samples,))?;
    let first = complement.call_method1("__mul__", (&left_samples,))?;
    first
        .call_method1("__add__", (&second,))?
        .call_method1("astype", (&float32,))
}
