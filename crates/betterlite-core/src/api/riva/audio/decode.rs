use std::fs;

use betterlite_pyhost::owned_temp::OwnedTempDir;
use pyo3::exceptions::PyImportError;
use pyo3::types::{PyAnyMethods as _, PyBytes, PyDict, PyDictMethods as _, PyList, PyModule};
use pyo3::{Bound, Py, PyAny, PyErr, PyResult, Python};

use super::error::{MAX_AUDIO_BYTES, RivaAudioError, call_failure};
use super::python::{as_int, decode_failure, exception_text, import_numpy, install_hint};

const NO_SAMPLES: &str = "Audio decode produced no samples.";

pub(super) struct DecodedAudio {
    pub(super) samples: Py<PyAny>,
    pub(super) source_rate: i64,
}

pub(super) fn decode(py: Python<'_>, file_bytes: &[u8]) -> Result<DecodedAudio, RivaAudioError> {
    let numpy = import_numpy(py)?;
    match try_soundfile(py, file_bytes)? {
        SoundfileAttempt::Decoded(decoded) => Ok(decoded),
        SoundfileAttempt::Failed(sf_error) => {
            decode_with_audioread(py, &numpy, file_bytes, &sf_error)
        }
    }
}

enum SoundfileAttempt {
    Decoded(DecodedAudio),
    Failed(String),
}

fn try_soundfile(py: Python<'_>, file_bytes: &[u8]) -> Result<SoundfileAttempt, RivaAudioError> {
    let soundfile = match py.import("soundfile") {
        Ok(module) => module,
        Err(error) => return Ok(SoundfileAttempt::Failed(exception_text(py, &error))),
    };
    match read_soundfile(py, &soundfile, file_bytes) {
        Ok(decoded) => Ok(SoundfileAttempt::Decoded(decoded)),
        Err(SoundfileError::TooLarge) => Err(RivaAudioError::TooLarge),
        Err(SoundfileError::Python(error)) => {
            Ok(SoundfileAttempt::Failed(exception_text(py, &error)))
        }
    }
}

enum SoundfileError {
    TooLarge,
    Python(PyErr),
}

fn read_soundfile(
    py: Python<'_>,
    soundfile: &Bound<'_, PyModule>,
    file_bytes: &[u8],
) -> Result<DecodedAudio, SoundfileError> {
    check_soundfile_size(py, soundfile, file_bytes)?;
    soundfile_samples(py, soundfile, file_bytes)
}

fn check_soundfile_size(
    py: Python<'_>,
    soundfile: &Bound<'_, PyModule>,
    file_bytes: &[u8],
) -> Result<(), SoundfileError> {
    let info = soundfile
        .getattr("info")
        .and_then(|info| info.call1((bytes_io(py, file_bytes)?,)))
        .map_err(SoundfileError::Python)?;
    let frames = info
        .getattr("frames")
        .and_then(|frames| frames.extract::<i64>())
        .map_err(SoundfileError::Python)?;
    let channels = info
        .getattr("channels")
        .and_then(|channels| channels.extract::<i64>())
        .map_err(SoundfileError::Python)?;
    if exceeds_limit(frames, channels) {
        return Err(SoundfileError::TooLarge);
    }
    Ok(())
}

fn soundfile_samples(
    py: Python<'_>,
    soundfile: &Bound<'_, PyModule>,
    file_bytes: &[u8],
) -> Result<DecodedAudio, SoundfileError> {
    let buffer = bytes_io(py, file_bytes).map_err(SoundfileError::Python)?;
    let keywords = PyDict::new(py);
    keywords
        .set_item("dtype", "float32")
        .map_err(SoundfileError::Python)?;
    keywords
        .set_item("always_2d", false)
        .map_err(SoundfileError::Python)?;
    let read = soundfile
        .getattr("read")
        .and_then(|read| read.call((&buffer,), Some(&keywords)))
        .map_err(SoundfileError::Python)?;
    let data = read.get_item(0_i32).map_err(SoundfileError::Python)?;
    let rate = read.get_item(1_i32).map_err(SoundfileError::Python)?;
    let source_rate = as_int(py, &rate).map_err(SoundfileError::Python)?;
    Ok(DecodedAudio {
        samples: data.unbind(),
        source_rate,
    })
}

fn exceeds_limit(frames: i64, channels: i64) -> bool {
    let limits = u64::try_from(frames).ok().zip(u64::try_from(channels).ok());
    limits.is_some_and(|(frame_count, channel_count)| {
        frame_count
            .checked_mul(channel_count)
            .and_then(|samples| samples.checked_mul(4))
            .is_none_or(|bytes| bytes > MAX_AUDIO_BYTES)
    })
}

fn bytes_io<'py>(py: Python<'py>, file_bytes: &[u8]) -> PyResult<Bound<'py, PyAny>> {
    py.import("io")?
        .getattr("BytesIO")?
        .call1((PyBytes::new(py, file_bytes),))
}

fn decode_with_audioread(
    py: Python<'_>,
    numpy: &Bound<'_, PyModule>,
    file_bytes: &[u8],
    sf_error: &str,
) -> Result<DecodedAudio, RivaAudioError> {
    let audioread = match py.import("audioread") {
        Ok(module) => module,
        Err(error) if error.is_instance_of::<PyImportError>(py) => {
            return Err(install_hint(sf_error));
        }
        Err(error) => return Err(call_failure(py, "import audioread", &error)),
    };
    let (_folder, path) = staged_file(file_bytes)?;
    let opened = audioread
        .getattr("audio_open")
        .and_then(|open| open.call1((path,)))
        .map_err(|error| decode_failure(py, &error))?;
    let source = opened
        .call_method0("__enter__")
        .map_err(|error| decode_failure(py, &error))?;
    let outcome = read_audioread(py, numpy, &source);
    let exited = opened
        .call_method1("__exit__", (py.None(), py.None(), py.None()))
        .map_err(|error| decode_failure(py, &error));
    match outcome {
        Ok(decoded) => {
            exited?;
            Ok(decoded)
        }
        Err(error) => {
            exited?;
            Err(error)
        }
    }
}

fn staged_file(file_bytes: &[u8]) -> Result<(OwnedTempDir, String), RivaAudioError> {
    let folder = OwnedTempDir::new()?;
    let path = folder.file("input.audio")?;
    fs::write(&path, file_bytes).map_err(|source| RivaAudioError::StageWrite { source })?;
    let text = path.to_str().ok_or_else(|| RivaAudioError::StagedPath {
        path: path.display().to_string(),
    })?;
    Ok((folder, text.to_owned()))
}

fn read_audioread(
    py: Python<'_>,
    numpy: &Bound<'_, PyModule>,
    source: &Bound<'_, PyAny>,
) -> Result<DecodedAudio, RivaAudioError> {
    let samplerate = source
        .getattr("samplerate")
        .and_then(|rate| as_int(py, &rate))
        .map_err(|error| decode_failure(py, &error))?;
    let channels = source
        .getattr("channels")
        .and_then(|channels| as_int(py, &channels))
        .map_err(|error| decode_failure(py, &error))?;
    let samples = audioread_samples(py, numpy, source, channels)?;
    Ok(DecodedAudio {
        samples: samples.unbind(),
        source_rate: samplerate,
    })
}

fn audioread_samples<'py>(
    py: Python<'py>,
    numpy: &Bound<'py, PyModule>,
    source: &Bound<'py, PyAny>,
    channels: i64,
) -> Result<Bound<'py, PyAny>, RivaAudioError> {
    let chunks = read_chunks(py, numpy, source)?;
    let interleaved = PyList::new(py, chunks)
        .and_then(|list| numpy.getattr("concatenate")?.call1((&list,)))
        .and_then(|joined| joined.call_method1("astype", ("float32",)))
        .and_then(|floats| floats.call_method1("__truediv__", (32768.0_f64,)))
        .map_err(|error| decode_failure(py, &error))?;
    if channels <= 1 {
        return Ok(interleaved);
    }
    interleaved
        .call_method1("reshape", (-1_i64, channels))
        .map_err(|error| decode_failure(py, &error))
}

fn read_chunks<'py>(
    py: Python<'py>,
    numpy: &Bound<'py, PyModule>,
    source: &Bound<'py, PyAny>,
) -> Result<Vec<Bound<'py, PyAny>>, RivaAudioError> {
    let fail = |error: PyErr| decode_failure(py, &error);
    let dtype = numpy.getattr("int16").map_err(fail)?;
    let frombuffer = numpy.getattr("frombuffer").map_err(fail)?;
    let mut chunks: Vec<Bound<'py, PyAny>> = Vec::new();
    let mut decoded_samples: u64 = 0;
    for item in source.try_iter().map_err(fail)? {
        let chunk = item.map_err(fail)?;
        let keywords = PyDict::new(py);
        keywords.set_item("dtype", &dtype).map_err(fail)?;
        let array = frombuffer.call((&chunk,), Some(&keywords)).map_err(fail)?;
        decoded_samples = added_samples(decoded_samples, chunk.len().map_err(fail)?)?;
        chunks.push(array);
    }
    if chunks.is_empty() {
        return Err(RivaAudioError::Decode {
            message: NO_SAMPLES.to_owned(),
        });
    }
    Ok(chunks)
}

fn added_samples(decoded_samples: u64, length: usize) -> Result<u64, RivaAudioError> {
    let samples = u64::try_from(length)
        .ok()
        .and_then(|bytes| bytes.checked_div(2))
        .ok_or(RivaAudioError::TooLarge)?;
    let total = decoded_samples
        .checked_add(samples)
        .ok_or(RivaAudioError::TooLarge)?;
    if total
        .checked_mul(4)
        .is_none_or(|bytes| bytes > MAX_AUDIO_BYTES)
    {
        return Err(RivaAudioError::TooLarge);
    }
    Ok(total)
}
