use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_router::fuse_presets::error::FusePresetsError;
use betterlite_router::fuse_presets::{
    PresetKind, ProfileText, get_fuse_presets, resolve_fuse_profile,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Catalog,
    Resolve {
        text: Option<String>,
        preset_id: Option<String>,
        kind: PresetKind,
    },
    ProfileText {
        text: String,
    },
}

#[derive(Debug, Serialize)]
struct TextAnswer {
    kind: TextKind,
    text: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum TextKind {
    Accepted,
    TooShort,
    TooLong,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Catalog => to_json(&*get_fuse_presets().or_config_error()?),
            Input::Resolve {
                text,
                preset_id,
                kind,
            } => {
                let profile: Option<ProfileText> = text
                    .as_deref()
                    .map(|value| ProfileText::try_from(value.to_owned()))
                    .transpose()
                    .or_config_error()?;
                let resolved = resolve_fuse_profile(profile.as_ref(), preset_id.as_deref(), *kind)
                    .or_config_error()?;
                to_json(resolved)
            }
            Input::ProfileText { text } => text_answer(text),
        }
    }
}

fn text_answer(text: &str) -> HelperResult {
    let profile = match ProfileText::try_from(text.to_owned()) {
        Ok(profile) => profile,
        Err(FusePresetsError::TextTooShort) => {
            return to_json(TextAnswer {
                kind: TextKind::TooShort,
                text: None,
            });
        }
        Err(FusePresetsError::TextTooLong { .. }) => {
            return to_json(TextAnswer {
                kind: TextKind::TooLong,
                text: None,
            });
        }
        // ProfileText::try_from builds only the two text errors, so no other variant can reach here
        Err(error @ (FusePresetsError::Catalog(_) | FusePresetsError::SourcesEmpty)) => {
            return Err(Box::new(HelperError {
                class: "ProfileTextError".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            }));
        }
    };
    let stripped: &str = profile.as_ref();
    to_json(TextAnswer {
        kind: TextKind::Accepted,
        text: Some(stripped.to_owned()),
    })
}

fn to_json<T>(value: T) -> HelperResult
where
    T: Serialize,
{
    serde_json::to_value(value).map_err(|error| {
        Box::new(HelperError {
            class: "SerializationError".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: error.to_string(),
        })
    })
}
