mod decode;
pub mod error;
mod python;
mod resample;

mod tests;

use std::time::Duration;

use betterlite_pyhost::blocking_work::WorkTracker;
use betterlite_pyhost::host::PythonHost;
use pyo3::types::{PyAnyMethods as _, PyDict, PyDictMethods as _, PySlice};
use pyo3::{Bound, PyAny, Python};
use rust_decimal::Decimal;

use self::error::{MAX_AUDIO_BYTES, RivaAudioError, call_failure};
use self::python::{as_int, import_numpy};

pub const TARGET_SAMPLE_RATE_HZ: u32 = 16_000;
pub const TARGET_CHANNEL_COUNT: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResampledAudio {
    pub pcm_bytes: Vec<u8>,
    pub duration_seconds: Decimal,
    pub sample_rate_hz: u32,
    pub num_channels: u32,
}

pub async fn decode_riva_audio(
    host: &PythonHost,
    tracker: &WorkTracker,
    deadline: Duration,
    file_bytes: Vec<u8>,
) -> Result<ResampledAudio, RivaAudioError> {
    match tracker
        .run(host, deadline, move |py| {
            decode_riva_audio_attached(py, &file_bytes)
        })
        .await
    {
        Ok(outcome) => outcome,
        Err(error) => Err(RivaAudioError::Work {
            message: error.to_string(),
        }),
    }
}

pub fn decode_riva_audio_attached(
    py: Python<'_>,
    file_bytes: &[u8],
) -> Result<ResampledAudio, RivaAudioError> {
    let size = u64::try_from(file_bytes.len()).map_err(|_| RivaAudioError::TooLarge)?;
    if size > MAX_AUDIO_BYTES {
        return Err(RivaAudioError::TooLarge);
    }
    let decoded = decode::decode(py, file_bytes)?;
    let source_rate = decoded.source_rate;
    let samples = downmix(py, decoded)?;
    let resampled = resample::resample(py, &samples, source_rate)?;
    pcm(&resampled)
}

fn downmix<'py>(
    py: Python<'py>,
    decoded: decode::DecodedAudio,
) -> Result<Bound<'py, PyAny>, RivaAudioError> {
    let numpy = import_numpy(py)?;
    let samples = mono(py, decoded.samples.into_bound(py))?;
    let keywords = PyDict::new(py);
    keywords
        .set_item(
            "dtype",
            numpy
                .getattr("float32")
                .map_err(|error| call_failure(py, "numpy.float32", &error))?,
        )
        .map_err(|error| call_failure(py, "numpy.asarray", &error))?;
    let array = numpy
        .getattr("asarray")
        .and_then(|asarray| asarray.call((&samples,), Some(&keywords)))
        .map_err(|error| call_failure(py, "numpy.asarray", &error))?;
    array
        .call_method0("ravel")
        .map_err(|error| call_failure(py, "samples.ravel", &error))
}

fn mono<'py>(
    py: Python<'py>,
    data: Bound<'py, PyAny>,
) -> Result<Bound<'py, PyAny>, RivaAudioError> {
    let ndim = data
        .getattr("ndim")
        .and_then(|ndim| as_int(py, &ndim))
        .map_err(|error| call_failure(py, "samples.ndim", &error))?;
    if ndim != 2 {
        return Ok(data);
    }
    let channels = data
        .getattr("shape")
        .and_then(|shape| shape.get_item(1))
        .and_then(|channels| as_int(py, &channels))
        .map_err(|error| call_failure(py, "samples.shape", &error))?;
    if channels > 1 {
        let keywords = PyDict::new(py);
        keywords
            .set_item("axis", 1_i64)
            .map_err(|error| call_failure(py, "samples.mean", &error))?;
        return data
            .call_method("mean", (), Some(&keywords))
            .map_err(|error| call_failure(py, "samples.mean", &error));
    }
    data.get_item((PySlice::full(py), 0_i64))
        .map_err(|error| call_failure(py, "samples[:, 0]", &error))
}

fn pcm(samples: &Bound<'_, PyAny>) -> Result<ResampledAudio, RivaAudioError> {
    let py = samples.py();
    let numpy = import_numpy(py)?;
    let keywords = PyDict::new(py);
    keywords
        .set_item("out", samples)
        .map_err(|error| call_failure(py, "numpy.clip", &error))?;
    numpy
        .getattr("clip")
        .and_then(|clip| clip.call((-1.0_f64, 1.0_f64), Some(&keywords)))
        .map_err(|error| call_failure(py, "numpy.clip", &error))?;
    let scaled = samples
        .call_method1("__mul__", (32767.0_f64,))
        .map_err(|error| call_failure(py, "samples * 32767.0", &error))?;
    let int16 = scaled
        .call_method1("astype", ("<i2",))
        .map_err(|error| call_failure(py, "samples.astype", &error))?;
    let count = int16
        .getattr("size")
        .and_then(|size| as_int(py, &size))
        .map_err(|error| call_failure(py, "pcm.size", &error))?;
    let pcm_bytes: Vec<u8> = int16
        .call_method0("tobytes")
        .and_then(|bytes| bytes.extract())
        .map_err(|error| call_failure(py, "pcm.tobytes", &error))?;
    let samples = u64::try_from(count).map_err(|_| RivaAudioError::Duration)?;
    let duration_seconds = Decimal::from(samples)
        .checked_div(Decimal::from(TARGET_SAMPLE_RATE_HZ))
        .ok_or(RivaAudioError::Duration)?;
    Ok(ResampledAudio {
        pcm_bytes,
        duration_seconds,
        sample_rate_hz: TARGET_SAMPLE_RATE_HZ,
        num_channels: TARGET_CHANNEL_COUNT,
    })
}
