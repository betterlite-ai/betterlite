#![cfg(test)]

use std::ffi::CString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use betterlite_pyhost::blocking_work::WorkTracker;
use betterlite_pyhost::host::{DEFAULT_SHUTDOWN_GRACE, HostConfig, PythonHost};
use pyo3::types::{PyAnyMethods as _, PyModule};
use pyo3::{Bound, Py, PyAny, Python};
use rust_decimal::Decimal;
use tokio::sync::Mutex;

use super::error::{MAX_AUDIO_BYTES, RivaAudioError};
use super::{ResampledAudio, TARGET_SAMPLE_RATE_HZ, decode_riva_audio, decode_riva_audio_attached};

// the interpreter and sys.modules are shared, so the tests run one at a time
static PYTHON: Mutex<()> = Mutex::const_new(());

const PROBE: &str = "events = []\nstaging = []\n";

const FAKE_MODULES: [&str; 7] = [
    "riva_audio_probe",
    "soundfile",
    "audioread",
    "soxr",
    "scipy",
    "scipy.signal",
    "scipy_signal",
];

const SCIPY_SIGNAL: &str = "import riva_audio_probe as probe
def resample_poly(samples, up, down):
    probe.events.append('scipy.resample_poly:' + str(up) + ':' + str(down))
    return samples
";

fn site_packages() -> Option<PathBuf> {
    let lib = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/oracle/litellm/.venv/lib");
    fs::read_dir(lib)
        .ok()?
        .flatten()
        .map(|entry| entry.path().join("site-packages"))
        .find(|folder| folder.is_dir())
}

fn oracle_host() -> Option<PythonHost> {
    PythonHost::start(&HostConfig {
        module_dirs: vec![site_packages()?],
        shutdown_grace: DEFAULT_SHUTDOWN_GRACE,
    })
    .ok()
}

fn host_with_numpy() -> Option<PythonHost> {
    if let Some(host) = oracle_host() {
        return Some(host);
    }
    let host = PythonHost::start(&HostConfig::default()).ok()?;
    host.import_check("numpy").ok()?;
    Some(host)
}

fn modules(py: Python<'_>) -> Bound<'_, PyAny> {
    py.import("sys").unwrap().getattr("modules").unwrap()
}

fn install<'py>(py: Python<'py>, name: &str, source: &str) -> Bound<'py, PyModule> {
    let source = CString::new(source).unwrap();
    let file_name = CString::new(format!("{name}.py")).unwrap();
    let module_name = CString::new(name).unwrap();
    PyModule::from_code(py, &source, &file_name, &module_name).unwrap()
}

fn install_scipy(py: Python<'_>) {
    let signal = install(py, "scipy_signal", SCIPY_SIGNAL);
    let scipy = PyModule::new(py, "scipy").unwrap();
    scipy.setattr("signal", &signal).unwrap();
    modules(py).set_item("scipy", &scipy).unwrap();
    modules(py).set_item("scipy.signal", &signal).unwrap();
}

// a fake for a module the environment would otherwise supply has to come back afterwards
struct Hidden<'host> {
    host: &'host PythonHost,
    name: &'static str,
    previous: Option<Py<PyAny>>,
}

impl<'host> Hidden<'host> {
    fn new(host: &'host PythonHost, name: &'static str) -> Self {
        let previous = host.attach(|py| {
            modules(py)
                .get_item(name)
                .ok()
                .map(|module| module.unbind())
        });
        host.attach(|py| modules(py).set_item(name, py.None()).unwrap());
        Self {
            host,
            name,
            previous,
        }
    }
}

impl Drop for Hidden<'_> {
    fn drop(&mut self) {
        let previous = self.previous.take();
        let name = self.name;
        self.host.attach(move |py| match previous {
            Some(module) => modules(py).set_item(name, module.bind(py)).unwrap(),
            None => drop(modules(py).del_item(name)),
        });
    }
}

fn reset_fakes(host: &PythonHost) {
    host.attach(|py| {
        for name in FAKE_MODULES {
            drop(modules(py).del_item(name));
        }
    });
}

fn probe_list(host: &PythonHost, name: &str) -> Vec<String> {
    host.attach(|py| {
        py.import("riva_audio_probe")
            .unwrap()
            .getattr(name)
            .unwrap()
            .extract()
            .unwrap()
    })
}

fn probe_events(host: &PythonHost) -> Vec<String> {
    probe_list(host, "events")
}

fn probe_staging(host: &PythonHost) -> Vec<String> {
    probe_list(host, "staging")
}

fn soundfile_source(data: &str, frames: i64, channels: i64, rate: i64) -> String {
    format!(
        "import numpy as np
class _Info:
    frames = {frames}
    channels = {channels}
    samplerate = {rate}
def info(file):
    return _Info()
def read(file, dtype='float32', always_2d=False):
    return (np.asarray({data}, dtype=np.float32), {rate})
"
    )
}

fn soundfile_failure(message: &str) -> String {
    format!(
        "class _Error(Exception):
    pass
def info(file):
    raise _Error('{message}')
def read(file, dtype='float32', always_2d=False):
    raise _Error('{message}')
"
    )
}

fn oversized_soundfile(frames: i64) -> String {
    format!(
        "class _Info:
    frames = {frames}
    channels = 1
    samplerate = 8000
def info(file):
    return _Info()
def read(file, dtype='float32', always_2d=False):
    raise RuntimeError('read must not run')
"
    )
}

#[derive(Clone, Copy)]
enum AudioreadFailure {
    None,
    Open,
    Iteration,
}

fn audioread_source(
    chunks: &str,
    samplerate: i64,
    channels: i64,
    failure: AudioreadFailure,
) -> String {
    let open = match failure {
        AudioreadFailure::Open => "raise RuntimeError('cannot open audio')",
        AudioreadFailure::None | AudioreadFailure::Iteration => "pass",
    };
    let iteration = match failure {
        AudioreadFailure::Iteration => "raise RuntimeError('cannot read audio')",
        AudioreadFailure::None | AudioreadFailure::Open => "pass",
    };
    format!(
        "import riva_audio_probe as probe
class _Source:
    samplerate = {samplerate}
    channels = {channels}
    def __enter__(self):
        return self
    def __exit__(self, *args):
        probe.events.append('audioread.exit')
    def __iter__(self):
        {iteration}
        return iter({chunks})
class _Open:
    def __call__(self, path):
        probe.staging.append(path)
        probe.events.append('audioread.open')
        {open}
        return _Source()
audio_open = _Open()
"
    )
}

fn soxr_source(failing: bool) -> String {
    let body = if failing {
        "raise RuntimeError('soxr failed')"
    } else {
        "return samples"
    };
    format!(
        "import riva_audio_probe as probe
def resample(samples, source_rate, target_rate):
    probe.events.append('soxr.resample:' + str(source_rate) + ':' + str(target_rate))
    {body}
"
    )
}

fn wav_bytes(samples: &[i16], channels: u16, rate: u32) -> Vec<u8> {
    let data_len = u32::try_from(samples.len().checked_mul(2).unwrap()).unwrap();
    let block_align = channels.checked_mul(2).unwrap();
    let byte_rate = rate.checked_mul(u32::from(block_align)).unwrap();
    let capacity = 44_usize
        .checked_add(samples.len().checked_mul(2).unwrap())
        .unwrap();
    let mut bytes = Vec::with_capacity(capacity);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&36_u32.checked_add(data_len).unwrap().to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16_u32.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&channels.to_le_bytes());
    bytes.extend_from_slice(&rate.to_le_bytes());
    bytes.extend_from_slice(&byte_rate.to_le_bytes());
    bytes.extend_from_slice(&block_align.to_le_bytes());
    bytes.extend_from_slice(&16_u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_len.to_le_bytes());
    for sample in samples {
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    bytes
}

fn pcm_values(audio: &ResampledAudio) -> Vec<i16> {
    audio
        .pcm_bytes
        .chunks_exact(2)
        .map(|pair| i16::from_le_bytes([pair[0], pair[1]]))
        .collect()
}

fn duration(samples: u64, rate: u32) -> Decimal {
    Decimal::from(samples)
        .checked_div(Decimal::from(rate))
        .unwrap()
}

fn sample_count(audio: &ResampledAudio) -> u64 {
    u64::try_from(audio.pcm_bytes.len().checked_div(2).unwrap()).unwrap()
}

fn decode(host: &PythonHost, file_bytes: &[u8]) -> Result<ResampledAudio, RivaAudioError> {
    host.attach(|py| decode_riva_audio_attached(py, file_bytes))
}

#[tokio::test]
async fn decodes_a_stereo_file_to_16k_mono_pcm() {
    let _guard = PYTHON.lock().await;
    let Some(host) = oracle_host() else {
        return;
    };
    reset_fakes(&host);
    let wav = wav_bytes(&[-0x4000, 0, 0x4000, 0], 2, 8000);
    let tracker = WorkTracker::new(1, Duration::from_secs(5));
    let audio = decode_riva_audio(&host, &tracker, Duration::from_secs(30), wav)
        .await
        .unwrap();
    assert_eq!(pcm_values(&audio), vec![-8191, -2730, 2730, 8191]);
    assert_eq!(audio.duration_seconds, duration(4, TARGET_SAMPLE_RATE_HZ));
    assert_eq!(audio.sample_rate_hz, TARGET_SAMPLE_RATE_HZ);
    assert_eq!(audio.num_channels, 1);
    assert_eq!(
        audio.pcm_bytes,
        vec![0x01, 0xe0, 0x56, 0xf5, 0xaa, 0x0a, 0xff, 0x1f]
    );
}

#[test]
fn passes_audio_at_the_target_rate_through_unchanged() {
    let _guard = PYTHON.blocking_lock();
    let Some(host) = oracle_host() else {
        return;
    };
    reset_fakes(&host);
    let audio = decode(&host, &wav_bytes(&[0x4000, -0x4000, 8192], 1, 16_000)).unwrap();
    assert_eq!(pcm_values(&audio), vec![16383, -16383, 8191]);
    assert_eq!(audio.duration_seconds, duration(3, TARGET_SAMPLE_RATE_HZ));
    assert_eq!(audio.sample_rate_hz, TARGET_SAMPLE_RATE_HZ);
    assert_eq!(audio.num_channels, 1);
}

#[test]
fn rounds_a_short_high_rate_file_down_to_one_sample() {
    let _guard = PYTHON.blocking_lock();
    let Some(host) = oracle_host() else {
        return;
    };
    reset_fakes(&host);
    let audio = decode(&host, &wav_bytes(&[0x4000], 1, 44_100)).unwrap();
    assert_eq!(pcm_values(&audio), vec![16383]);
    assert_eq!(audio.duration_seconds, duration(1, TARGET_SAMPLE_RATE_HZ));
}

#[test]
fn falls_back_to_audioread_when_soundfile_fails() {
    let _guard = PYTHON.blocking_lock();
    let Some(host) = host_with_numpy() else {
        return;
    };
    reset_fakes(&host);
    host.attach(|py| {
        install(py, "riva_audio_probe", PROBE);
        install(
            py,
            "soundfile",
            &soundfile_failure("cannot open sound file"),
        );
        install(
            py,
            "audioread",
            &audioread_source("[bytes.fromhex('00c0')]", 16_000, 1, AudioreadFailure::None),
        );
    });
    let audio = decode(&host, b"staged bytes").unwrap();
    assert_eq!(pcm_values(&audio), vec![-16383]);
    assert_eq!(audio.duration_seconds, duration(1, TARGET_SAMPLE_RATE_HZ));
    let staging = probe_staging(&host);
    assert_eq!(staging.len(), 1);
    assert!(staging[0].ends_with(".audio"));
    assert!(!Path::new(&staging[0]).exists());
    assert_eq!(
        probe_events(&host),
        vec!["audioread.open", "audioread.exit"]
    );
}

#[test]
fn downmixes_stereo_audioread_chunks() {
    let _guard = PYTHON.blocking_lock();
    let Some(host) = host_with_numpy() else {
        return;
    };
    reset_fakes(&host);
    host.attach(|py| {
        install(py, "riva_audio_probe", PROBE);
        install(
            py,
            "soundfile",
            &soundfile_failure("cannot open sound file"),
        );
        install(
            py,
            "audioread",
            &audioread_source(
                "[bytes.fromhex('e80318fc'), bytes.fromhex('d00730f8')]",
                16_000,
                2,
                AudioreadFailure::None,
            ),
        );
    });
    let audio = decode(&host, b"staged bytes").unwrap();
    assert_eq!(pcm_values(&audio), vec![0, 0]);
    assert_eq!(audio.duration_seconds, duration(2, TARGET_SAMPLE_RATE_HZ));
}

#[test]
fn rejects_an_empty_audioread_decode() {
    let _guard = PYTHON.blocking_lock();
    let Some(host) = host_with_numpy() else {
        return;
    };
    reset_fakes(&host);
    host.attach(|py| {
        install(py, "riva_audio_probe", PROBE);
        install(
            py,
            "soundfile",
            &soundfile_failure("cannot open sound file"),
        );
        install(
            py,
            "audioread",
            &audioread_source("[]", 16_000, 1, AudioreadFailure::None),
        );
    });
    let error = decode(&host, b"staged bytes").unwrap_err();
    assert!(matches!(error, RivaAudioError::Decode { .. }));
    assert_eq!(error.to_string(), "Audio decode produced no samples.");
    assert_eq!(error.response().status_code, 400);
    assert_eq!(
        probe_events(&host),
        vec!["audioread.open", "audioread.exit"]
    );
}

#[test]
fn wraps_audioread_open_failures() {
    let _guard = PYTHON.blocking_lock();
    let Some(host) = host_with_numpy() else {
        return;
    };
    reset_fakes(&host);
    host.attach(|py| {
        install(py, "riva_audio_probe", PROBE);
        install(
            py,
            "soundfile",
            &soundfile_failure("cannot open sound file"),
        );
        install(
            py,
            "audioread",
            &audioread_source("[]", 16_000, 1, AudioreadFailure::Open),
        );
    });
    let error = decode(&host, b"staged bytes").unwrap_err();
    assert_eq!(
        error.to_string(),
        "Could not decode audio for Riva STT. Convert your audio to wav/flac/ogg before calling the API. Underlying error: cannot open audio"
    );
    assert_eq!(error.response().status_code, 400);
    assert_eq!(probe_events(&host), vec!["audioread.open"]);
    let staging = probe_staging(&host);
    assert_eq!(staging.len(), 1);
    assert!(!Path::new(&staging[0]).exists());
}

#[test]
fn wraps_audioread_read_failures() {
    let _guard = PYTHON.blocking_lock();
    let Some(host) = host_with_numpy() else {
        return;
    };
    reset_fakes(&host);
    host.attach(|py| {
        install(py, "riva_audio_probe", PROBE);
        install(
            py,
            "soundfile",
            &soundfile_failure("cannot open sound file"),
        );
        install(
            py,
            "audioread",
            &audioread_source("[]", 16_000, 1, AudioreadFailure::Iteration),
        );
    });
    let error = decode(&host, b"staged bytes").unwrap_err();
    assert_eq!(
        error.to_string(),
        "Could not decode audio for Riva STT. Convert your audio to wav/flac/ogg before calling the API. Underlying error: cannot read audio"
    );
    assert_eq!(
        probe_events(&host),
        vec!["audioread.open", "audioread.exit"]
    );
    let staging = probe_staging(&host);
    assert_eq!(staging.len(), 1);
    assert!(!Path::new(&staging[0]).exists());
}

#[test]
fn reports_the_soundfile_error_when_audioread_is_missing() {
    let _guard = PYTHON.blocking_lock();
    let Some(host) = host_with_numpy() else {
        return;
    };
    reset_fakes(&host);
    host.attach(|py| {
        install(py, "riva_audio_probe", PROBE);
        install(
            py,
            "soundfile",
            &soundfile_failure("cannot open sound file"),
        );
    });
    let _audioread = Hidden::new(&host, "audioread");
    let error = decode(&host, b"staged bytes").unwrap_err();
    assert_eq!(
        error.to_string(),
        "Could not decode audio for Riva STT. Install audio extras (`pip install 'litellm[stt-nvidia-riva]'`) or convert your audio to wav/flac/ogg before calling the API. Underlying error: cannot open sound file"
    );
    assert_eq!(error.response().status_code, 400);
    assert_eq!(error.response().error_type, "invalid_request_error");
    assert!(probe_events(&host).is_empty());
}

#[test]
fn reports_missing_numpy_with_the_install_hint() {
    let _guard = PYTHON.blocking_lock();
    let host = PythonHost::start(&HostConfig::default()).unwrap();
    let _numpy = Hidden::new(&host, "numpy");
    let error = decode(&host, b"anything").unwrap_err();
    assert!(matches!(error, RivaAudioError::NumpyMissing));
    assert_eq!(
        error.to_string(),
        "numpy is required for Riva audio resampling. Install Riva STT extras to enable automatic audio resampling: `pip install 'litellm[stt-nvidia-riva]'`"
    );
    assert_eq!(error.response().status_code, 500);
    assert_eq!(error.response().error_type, "internal_server_error");
}

#[test]
fn selects_the_first_column_of_a_single_channel_matrix() {
    let _guard = PYTHON.blocking_lock();
    let Some(host) = host_with_numpy() else {
        return;
    };
    reset_fakes(&host);
    host.attach(|py| {
        install(py, "riva_audio_probe", PROBE);
        install(
            py,
            "soundfile",
            &soundfile_source("[[0.5], [-0.25]]", 2, 1, 16_000),
        );
    });
    let audio = decode(&host, b"fake audio").unwrap();
    assert_eq!(pcm_values(&audio), vec![16383, -8191]);
}

#[test]
fn prefers_soxr_over_scipy() {
    let _guard = PYTHON.blocking_lock();
    let Some(host) = host_with_numpy() else {
        return;
    };
    reset_fakes(&host);
    host.attach(|py| {
        install(py, "riva_audio_probe", PROBE);
        install(
            py,
            "soundfile",
            &soundfile_source("[0.0, 0.0, 0.25, 0.0]", 4, 1, 8_000),
        );
        install(py, "soxr", &soxr_source(false));
        install_scipy(py);
    });
    let audio = decode(&host, b"fake audio").unwrap();
    assert_eq!(pcm_values(&audio), vec![0, 0, 8191, 0]);
    assert_eq!(probe_events(&host), vec!["soxr.resample:8000:16000"]);
}

#[test]
fn does_not_fall_back_when_soxr_fails() {
    let _guard = PYTHON.blocking_lock();
    let Some(host) = host_with_numpy() else {
        return;
    };
    reset_fakes(&host);
    host.attach(|py| {
        install(py, "riva_audio_probe", PROBE);
        install(
            py,
            "soundfile",
            &soundfile_source("[0.0, 0.0, 0.25, 0.0]", 4, 1, 8_000),
        );
        install(py, "soxr", &soxr_source(true));
        install_scipy(py);
    });
    let error = decode(&host, b"fake audio").unwrap_err();
    assert!(matches!(
        error,
        RivaAudioError::Python {
            operation: "soxr.resample",
            ..
        }
    ));
    assert_eq!(error.response().status_code, 500);
    assert_eq!(error.response().error_type, "internal_server_error");
    assert_eq!(
        error.to_string(),
        "soxr.resample failed: RuntimeError: soxr failed"
    );
    assert_eq!(probe_events(&host), vec!["soxr.resample:8000:16000"]);
}

#[test]
fn uses_scipy_when_soxr_is_missing() {
    let _guard = PYTHON.blocking_lock();
    let Some(host) = host_with_numpy() else {
        return;
    };
    reset_fakes(&host);
    host.attach(|py| {
        install(py, "riva_audio_probe", PROBE);
        install(
            py,
            "soundfile",
            &soundfile_source("[0.0, 0.0, 0.25, 0.0]", 4, 1, 8_000),
        );
        install_scipy(py);
    });
    let _soxr = Hidden::new(&host, "soxr");
    let audio = decode(&host, b"fake audio").unwrap();
    assert_eq!(pcm_values(&audio), vec![0, 0, 8191, 0]);
    assert_eq!(probe_events(&host), vec!["scipy.resample_poly:2:1"]);
}

#[test]
fn computes_scipy_factors_from_the_greatest_common_divisor() {
    let _guard = PYTHON.blocking_lock();
    let Some(host) = host_with_numpy() else {
        return;
    };
    reset_fakes(&host);
    host.attach(|py| {
        install(py, "riva_audio_probe", PROBE);
        install(
            py,
            "soundfile",
            &soundfile_source("[0.1, 0.5, 0.9]", 3, 1, 44_100),
        );
        install_scipy(py);
    });
    let _soxr = Hidden::new(&host, "soxr");
    let audio = decode(&host, b"fake audio").unwrap();
    assert_eq!(pcm_values(&audio), vec![3276, 29490]);
    assert_eq!(probe_events(&host), vec!["scipy.resample_poly:160:441"]);
}

#[test]
fn uses_the_linear_fallback_when_no_resampler_is_installed() {
    let _guard = PYTHON.blocking_lock();
    let Some(host) = host_with_numpy() else {
        return;
    };
    reset_fakes(&host);
    host.attach(|py| {
        install(py, "riva_audio_probe", PROBE);
        install(
            py,
            "soundfile",
            &soundfile_source("[-0.5, 0.0]", 2, 1, 8_000),
        );
    });
    let _soxr = Hidden::new(&host, "soxr");
    let _scipy = Hidden::new(&host, "scipy");
    let audio = decode(&host, b"fake audio").unwrap();
    assert_eq!(pcm_values(&audio), vec![-16383, -10922, -5461, 0]);
    assert_eq!(audio.duration_seconds, duration(4, TARGET_SAMPLE_RATE_HZ));
    assert!(probe_events(&host).is_empty());
}

#[test]
fn rounds_the_linear_target_length_half_even() {
    let _guard = PYTHON.blocking_lock();
    let Some(host) = host_with_numpy() else {
        return;
    };
    reset_fakes(&host);
    host.attach(|py| {
        install(py, "riva_audio_probe", PROBE);
        install(
            py,
            "soundfile",
            &soundfile_source("[0.0, 0.25, 0.5, 0.75, 1.0]", 5, 1, 32_000),
        );
    });
    let _soxr = Hidden::new(&host, "soxr");
    let _scipy = Hidden::new(&host, "scipy");
    let audio = decode(&host, b"fake audio").unwrap();
    assert_eq!(pcm_values(&audio), vec![0, 0x7FFF]);
}

#[test]
fn keeps_negative_full_scale_at_minus_32767() {
    let _guard = PYTHON.blocking_lock();
    let Some(host) = host_with_numpy() else {
        return;
    };
    reset_fakes(&host);
    host.attach(|py| {
        install(py, "riva_audio_probe", PROBE);
        install(
            py,
            "soundfile",
            &soundfile_source("[-1.0, 1.0]", 2, 1, 8_000),
        );
    });
    let _soxr = Hidden::new(&host, "soxr");
    let _scipy = Hidden::new(&host, "scipy");
    let audio = decode(&host, b"fake audio").unwrap();
    assert_eq!(pcm_values(&audio), vec![-0x7FFF, -10922, 10922, 0x7FFF]);
}

#[test]
fn skips_resampling_for_an_empty_decode() {
    let _guard = PYTHON.blocking_lock();
    let Some(host) = host_with_numpy() else {
        return;
    };
    reset_fakes(&host);
    host.attach(|py| {
        install(py, "riva_audio_probe", PROBE);
        install(py, "soundfile", &soundfile_source("[]", 0, 1, 8_000));
        install(py, "soxr", &soxr_source(true));
    });
    let audio = decode(&host, b"fake audio").unwrap();
    assert!(audio.pcm_bytes.is_empty());
    assert_eq!(audio.duration_seconds, Decimal::ZERO);
    assert!(probe_events(&host).is_empty());
}

#[test]
fn skips_resampling_at_the_target_rate() {
    let _guard = PYTHON.blocking_lock();
    let Some(host) = host_with_numpy() else {
        return;
    };
    reset_fakes(&host);
    host.attach(|py| {
        install(py, "riva_audio_probe", PROBE);
        install(py, "soundfile", &soundfile_source("[0.5]", 1, 1, 16_000));
        install(py, "soxr", &soxr_source(true));
    });
    let audio = decode(&host, b"fake audio").unwrap();
    assert_eq!(pcm_values(&audio), vec![16383]);
    assert!(probe_events(&host).is_empty());
}

#[test]
fn rejects_supplied_audio_over_the_limit() {
    let _guard = PYTHON.blocking_lock();
    let host = PythonHost::start(&HostConfig::default()).unwrap();
    let bytes = vec![
        0_u8;
        usize::try_from(MAX_AUDIO_BYTES)
            .unwrap()
            .checked_add(1)
            .unwrap()
    ];
    let error = decode(&host, &bytes).unwrap_err();
    assert!(matches!(error, RivaAudioError::TooLarge));
    assert_eq!(
        error.to_string(),
        "the audio input exceeds the 64 MiB limit for Riva STT"
    );
    let response = error.response();
    assert_eq!(response.status_code, 413);
    assert_eq!(response.error_type, "invalid_request_error");
    assert_eq!(response.param, Some("file"));
    assert_eq!(response.code, Some("audio_too_large"));
}

#[test]
fn accepts_supplied_audio_of_exactly_the_limit() {
    let _guard = PYTHON.blocking_lock();
    let Some(host) = host_with_numpy() else {
        return;
    };
    reset_fakes(&host);
    host.attach(|py| {
        install(py, "riva_audio_probe", PROBE);
        install(
            py,
            "soundfile",
            &soundfile_failure("cannot open sound file"),
        );
        install(
            py,
            "audioread",
            &audioread_source("[bytes.fromhex('00c0')]", 16_000, 1, AudioreadFailure::None),
        );
    });
    let bytes = vec![0_u8; usize::try_from(MAX_AUDIO_BYTES).unwrap()];
    let audio = decode(&host, &bytes).unwrap();
    assert_eq!(pcm_values(&audio), vec![-16383]);
}

#[test]
fn rejects_decoded_soundfile_audio_over_the_limit() {
    let _guard = PYTHON.blocking_lock();
    let Some(host) = host_with_numpy() else {
        return;
    };
    reset_fakes(&host);
    host.attach(|py| {
        install(py, "riva_audio_probe", PROBE);
        install(py, "soundfile", &oversized_soundfile(0x0100_0001));
    });
    let error = decode(&host, b"fake audio").unwrap_err();
    assert!(matches!(error, RivaAudioError::TooLarge));
    assert_eq!(error.response().status_code, 413);
}

#[test]
fn accepts_decoded_soundfile_audio_of_exactly_the_limit() {
    let _guard = PYTHON.blocking_lock();
    let Some(host) = host_with_numpy() else {
        return;
    };
    reset_fakes(&host);
    host.attach(|py| {
        install(py, "riva_audio_probe", PROBE);
        install(py, "soundfile", &oversized_soundfile(0x0100_0000));
        install(
            py,
            "audioread",
            &audioread_source("[bytes.fromhex('00c0')]", 16_000, 1, AudioreadFailure::None),
        );
    });
    let audio = decode(&host, b"fake audio").unwrap();
    assert_eq!(pcm_values(&audio), vec![-16383]);
    assert_eq!(
        probe_events(&host),
        vec!["audioread.open", "audioread.exit"]
    );
}

#[test]
fn rejects_audioread_output_over_the_limit() {
    let _guard = PYTHON.blocking_lock();
    let Some(host) = host_with_numpy() else {
        return;
    };
    reset_fakes(&host);
    host.attach(|py| {
        install(py, "riva_audio_probe", PROBE);
        install(
            py,
            "soundfile",
            &soundfile_failure("cannot open sound file"),
        );
        install(
            py,
            "audioread",
            &audioread_source("[bytes(33554436)]", 16_000, 1, AudioreadFailure::None),
        );
    });
    let error = decode(&host, b"staged bytes").unwrap_err();
    assert!(matches!(error, RivaAudioError::TooLarge));
    assert_eq!(error.response().status_code, 413);
    assert_eq!(
        probe_events(&host),
        vec!["audioread.open", "audioread.exit"]
    );
}

#[test]
fn accepts_audioread_output_of_exactly_the_limit() {
    let _guard = PYTHON.blocking_lock();
    let Some(host) = host_with_numpy() else {
        return;
    };
    reset_fakes(&host);
    host.attach(|py| {
        install(py, "riva_audio_probe", PROBE);
        install(
            py,
            "soundfile",
            &soundfile_failure("cannot open sound file"),
        );
        install(
            py,
            "audioread",
            &audioread_source(
                "[bytes(33554432), 'not-a-buffer']",
                16_000,
                1,
                AudioreadFailure::None,
            ),
        );
    });
    let error = decode(&host, b"staged bytes").unwrap_err();
    assert!(matches!(error, RivaAudioError::Decode { .. }));
    assert_eq!(error.response().status_code, 400);
    assert!(
        error
            .to_string()
            .contains("Underlying error: a bytes-like object is required")
    );
}

#[test]
fn replays_the_riva_audio_corpus() {
    let _guard = PYTHON.blocking_lock();
    let Some(host) = host_with_numpy() else {
        return;
    };
    reset_fakes(&host);
    if host.import_check("soundfile").is_err() {
        return;
    }
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/riva_audio");
    let (valid, broken) = fs::read_dir(&folder)
        .unwrap()
        .try_fold((0_usize, 0_usize), |(valid, broken), entry| {
            let is_valid = replay_corpus_file(&host, &entry?.path())?;
            let counts = if is_valid {
                (valid.checked_add(1).unwrap(), broken)
            } else {
                (valid, broken.checked_add(1).unwrap())
            };
            Ok::<(usize, usize), io::Error>(counts)
        })
        .unwrap();
    assert!(valid > 0, "no valid corpus files ran");
    assert!(broken > 0, "no broken corpus files ran");
}

fn replay_corpus_file(host: &PythonHost, path: &Path) -> io::Result<bool> {
    let name = path.file_name().unwrap().to_str().unwrap().to_owned();
    let bytes = fs::read(path)?;
    let outcome = decode(host, &bytes);
    if name.starts_with("valid-") {
        let audio = outcome.unwrap();
        assert_eq!(audio.sample_rate_hz, TARGET_SAMPLE_RATE_HZ, "{name}");
        assert_eq!(audio.num_channels, 1, "{name}");
        assert!(audio.pcm_bytes.len().is_multiple_of(2), "{name}");
        assert_eq!(
            audio.duration_seconds,
            duration(sample_count(&audio), TARGET_SAMPLE_RATE_HZ),
            "{name}"
        );
        return Ok(true);
    }
    let error = outcome.unwrap_err();
    assert_eq!(error.response().status_code, 400, "{name}: {error}");
    Ok(false)
}
