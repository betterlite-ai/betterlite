use std::fs::{self, File};
use std::io::{self, Read as _};
use std::path::{Component, Path, PathBuf};
use std::str::{self, Utf8Error};

use super::{
    Driver, Frame, MAX_CAPTURE_BYTES, MAX_EXECUTION_SECONDS, MAX_FRAME_BYTES, MAX_FRAMES,
    OracleLimits, WebSocketCase,
};

#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    #[error("invalid WebSocket UTF-8: {0}")]
    Utf8(#[from] Utf8Error),
    #[error("invalid WebSocket YAML: {0}")]
    Parse(#[source] Box<serde_saphyr::Error>),
    #[error("invalid WebSocket transcript: {0}")]
    Invalid(String),
    #[error("cannot read binary fixture: {0}")]
    Fixture(#[from] io::Error),
}

pub trait FixtureRepository {
    fn resolve(&self, path: &str, budget: usize) -> Result<Vec<u8>, DecodeError>;
}

pub struct FilesystemFixtures {
    pub root: PathBuf,
}

fn ensure(ok: bool, message: &str) -> Result<(), DecodeError> {
    ok.then_some(())
        .ok_or_else(|| DecodeError::Invalid(message.to_owned()))
}

impl FixtureRepository for FilesystemFixtures {
    fn resolve(&self, path: &str, budget: usize) -> Result<Vec<u8>, DecodeError> {
        let relative = Path::new(path);
        validate_binary_fixture(relative)?;
        let root = fs::canonicalize(&self.root)?;
        let candidate = fs::canonicalize(root.join(relative))?;
        ensure(
            candidate.starts_with(&root),
            "binary fixture resolves outside its root",
        )?;
        let metadata = fs::metadata(&candidate)?;
        let cap = usize::try_from(metadata.len()).unwrap_or(usize::MAX);
        ensure(cap <= budget, "captured traffic exceeds limit")?;
        let file = File::open(candidate)?;
        let mut bytes = Vec::new();
        file.take(u64::try_from(budget).unwrap_or(u64::MAX).saturating_add(1))
            .read_to_end(&mut bytes)?;
        Ok(bytes)
    }
}

pub fn decode_ws_case(
    bytes: &[u8],
    limits: &OracleLimits,
    fixtures: &dyn FixtureRepository,
) -> Result<WebSocketCase, DecodeError> {
    ensure(
        bytes.len()
            <= limits
                .max_capture_bytes
                .min(MAX_CAPTURE_BYTES)
                .saturating_add(0x0020_0000),
        "transcript input exceeds safety limit",
    )?;
    let text = str::from_utf8(bytes)?;
    let mut case: WebSocketCase =
        serde_saphyr::from_str(text).map_err(|source| DecodeError::Parse(Box::new(source)))?;
    validate_case(&case, limits)?;
    let frame_cap = limits.max_frame_bytes.min(MAX_FRAME_BYTES);
    let capture_cap = limits.max_capture_bytes.min(MAX_CAPTURE_BYTES);
    let mut traffic = 0_usize;
    for frames in [&mut case.frames, &mut case.input.frames] {
        resolve_frames(frames, frame_cap, capture_cap, fixtures, &mut traffic)?;
    }
    Ok(case)
}

fn validate_case(case: &WebSocketCase, limits: &OracleLimits) -> Result<(), DecodeError> {
    let max_frames = limits.max_frames.min(MAX_FRAMES);
    ensure(
        case.kind == "websocket"
            && case.path.starts_with('/')
            && (100..=599).contains(&case.expected_http_status),
        "invalid discriminator, path or handshake status",
    )?;
    ensure(
        case.frames.len() <= max_frames
            && case.input.frames.len() <= max_frames
            && case.execution_seconds <= limits.max_execution_seconds.min(MAX_EXECUTION_SECONDS),
        "frame count or execution exceeds limit",
    )?;
    ensure(
        (case.driver == Driver::Gateway) != case.service.is_some(),
        "driver and service selection disagree",
    )?;
    ensure(
        case.close.is_none() || case.expected_http_status == 101,
        "invalid close initiator or pre-upgrade close",
    )
}

fn resolve_frames(
    frames: &mut [Frame],
    frame_cap: usize,
    capture_cap: usize,
    fixtures: &dyn FixtureRepository,
    traffic: &mut usize,
) -> Result<(), DecodeError> {
    for frame in frames {
        let remaining = capture_cap
            .checked_sub(*traffic)
            .ok_or_else(|| DecodeError::Invalid("captured traffic exceeds limit".to_owned()))?;
        let budget = frame_cap.min(remaining);
        let size = match (&frame.text, &frame.binary) {
            (Some(text), None) => text.len(),
            (None, Some(path)) => {
                let data = fixtures.resolve(path, budget)?;
                let size = data.len();
                frame.binary_data = Some(data);
                size
            }
            (Some(_), Some(_)) | (None, None) => {
                return Err(DecodeError::Invalid(
                    "frame needs exactly one payload".to_owned(),
                ));
            }
        };
        ensure(size <= frame_cap, "frame exceeds size limit")?;
        *traffic = traffic.saturating_add(size);
        ensure(*traffic <= capture_cap, "captured traffic exceeds limit")?;
    }
    Ok(())
}

pub fn validate_binary_fixture(path: &Path) -> Result<(), DecodeError> {
    ensure(
        !path.as_os_str().is_empty()
            && path
                .components()
                .all(|part| matches!(part, Component::Normal(_))),
        "binary fixture path escapes its root",
    )
}
