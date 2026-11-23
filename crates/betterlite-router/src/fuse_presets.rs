pub mod error;

use std::sync::{Arc, Mutex, PoisonError};

use serde::{Deserialize, Serialize};

use error::FusePresetsError;

const BUNDLED_CATALOG: &str = include_str!("fuse_presets/fuse_presets.json");
const MAX_PROFILE_TEXT_CHARACTERS: usize = 4000;

// LiteLLM parses the bundled file once per process behind lru_cache(maxsize=1), and it never changes
static CATALOG: Mutex<Option<Arc<FusePresetCatalog>>> = Mutex::new(None);

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(try_from = "String")]
pub struct ProfileText(String);

impl TryFrom<String> for ProfileText {
    type Error = FusePresetsError;

    fn try_from(raw: String) -> Result<Self, FusePresetsError> {
        let text = raw.trim();
        if text.is_empty() {
            return Err(FusePresetsError::TextTooShort);
        }
        let length = text.chars().count();
        if length > MAX_PROFILE_TEXT_CHARACTERS {
            return Err(FusePresetsError::TextTooLong {
                length,
                limit: MAX_PROFILE_TEXT_CHARACTERS,
            });
        }
        Ok(Self(text.to_owned()))
    }
}

impl AsRef<str> for ProfileText {
    fn as_ref(&self) -> &str {
        self.0.as_str()
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(try_from = "RawFuseModelPreset")]
pub struct FuseModelPreset {
    pub id: String,
    pub label: String,
    pub text: ProfileText,
    pub sources: Vec<String>,
    pub model: String,
}

// try_from owns the public Deserialize impl, so extra="forbid" lives on this twin
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFuseModelPreset {
    id: String,
    label: String,
    text: ProfileText,
    sources: Vec<String>,
    model: String,
}

impl TryFrom<RawFuseModelPreset> for FuseModelPreset {
    type Error = FusePresetsError;

    fn try_from(raw: RawFuseModelPreset) -> Result<Self, FusePresetsError> {
        let RawFuseModelPreset {
            id,
            label,
            text,
            sources,
            model,
        } = raw;
        Ok(Self {
            id,
            label,
            text,
            sources: checked_sources(sources)?,
            model,
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(try_from = "RawFuseHarnessPreset")]
pub struct FuseHarnessPreset {
    pub id: String,
    pub label: String,
    pub text: ProfileText,
    pub sources: Vec<String>,
}

// try_from owns the public Deserialize impl, so extra="forbid" lives on this twin
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFuseHarnessPreset {
    id: String,
    label: String,
    text: ProfileText,
    sources: Vec<String>,
}

impl TryFrom<RawFuseHarnessPreset> for FuseHarnessPreset {
    type Error = FusePresetsError;

    fn try_from(raw: RawFuseHarnessPreset) -> Result<Self, FusePresetsError> {
        let RawFuseHarnessPreset {
            id,
            label,
            text,
            sources,
        } = raw;
        Ok(Self {
            id,
            label,
            text,
            sources: checked_sources(sources)?,
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FusePresetCatalog {
    pub version: String,
    pub models: Vec<FuseModelPreset>,
    pub harnesses: Vec<FuseHarnessPreset>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PresetKind {
    Model,
    Harness,
}

pub fn get_fuse_presets() -> Result<Arc<FusePresetCatalog>, FusePresetsError> {
    // a poisoned lock only guarded the one Option assignment below, so the shared catalog is still whole
    let mut stored = CATALOG.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(shared) = stored.as_ref() {
        return Ok(Arc::clone(shared));
    }
    let catalog = Arc::new(serde_json::from_str::<FusePresetCatalog>(BUNDLED_CATALOG)?);
    *stored = Some(Arc::clone(&catalog));
    Ok(catalog)
}

pub fn resolve_fuse_profile(
    text: Option<&ProfileText>,
    preset_id: Option<&str>,
    kind: PresetKind,
) -> Result<Option<ProfileText>, FusePresetsError> {
    let Some(preset_id) = preset_id else {
        return Ok(text.cloned());
    };
    let catalog = get_fuse_presets()?;
    // each kind searches only its own list, so a harness id asked for as a model finds nothing
    let found = match kind {
        PresetKind::Model => catalog
            .models
            .iter()
            .find(|entry| entry.id == preset_id)
            .map(|entry| entry.text.clone()),
        PresetKind::Harness => catalog
            .harnesses
            .iter()
            .find(|entry| entry.id == preset_id)
            .map(|entry| entry.text.clone()),
    };
    let Some(preset_text) = found else {
        return Ok(None);
    };
    Ok(Some(text.cloned().unwrap_or(preset_text)))
}

fn checked_sources(sources: Vec<String>) -> Result<Vec<String>, FusePresetsError> {
    if sources.is_empty() {
        return Err(FusePresetsError::SourcesEmpty);
    }
    Ok(sources)
}

mod tests;
