use serde::Deserialize;

pub(crate) const MAX_FRAMES: usize = 32;
pub(crate) const MAX_TEXT_BYTES: usize = 16 * 1024;
pub(crate) const MAX_TOTAL_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CloseDocument {
    pub(crate) code: u16,
    pub(crate) reason: String,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProviderDocument {
    pub(crate) requires_session_configuration: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) provider: Option<ProviderDocument>,
    pub(crate) frames: Vec<String>,
    pub(crate) close: Option<CloseDocument>,
    pub(crate) pending: Vec<String>,
    pub(crate) force_transcription_model: Option<String>,
}

impl Input {
    pub(crate) fn check(&self) -> Result<(), String> {
        let sizes = self
            .frames
            .iter()
            .map(String::len)
            .chain(self.pending.iter().map(String::len));
        if self.frames.len() > MAX_FRAMES
            || self.pending.len() > MAX_FRAMES
            || sizes.clone().any(|size| size > MAX_TEXT_BYTES)
            || sizes.fold(0_usize, usize::saturating_add) > MAX_TOTAL_BYTES
        {
            return Err(String::from("the scripted relay input is past its limits"));
        }
        Ok(())
    }
}
