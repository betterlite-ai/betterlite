use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;
use crate::sections::Section;

mod tests;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    model_list: Vec<RawDeployment>,
}

#[derive(Debug, Default, Deserialize)]
struct RawDeployment {
    #[serde(default)]
    model_info: RawInfo,
}

#[derive(Debug, Default, Deserialize)]
struct RawInfo {
    access_windows: Option<Vec<RawWindow>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawWindow {
    start: Option<String>,
    end: Option<String>,
    timezone: Option<String>,
    team_ids: Option<Vec<String>>,
}

// One configured window with its environment references resolved; the router validates the text into times and a zone.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WindowConfig {
    pub start: Option<String>,
    pub end: Option<String>,
    pub timezone: Option<String>,
    pub team_ids: Option<Vec<String>>,
}

// index-aligned with model_list, which the deployment catalog keeps in config order
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Settings {
    pub deployments: Vec<Option<Vec<WindowConfig>>>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let mut deployments = Vec::with_capacity(self.model_list.len());
        for (index, deployment) in self.model_list.into_iter().enumerate() {
            deployments.push(configured(
                index,
                deployment.model_info.access_windows,
                lookup,
            )?);
        }
        Ok(Settings { deployments })
    }
}

fn configured<L>(
    index: usize,
    raw: Option<Vec<RawWindow>>,
    lookup: &L,
) -> Result<Option<Vec<WindowConfig>>, ConfigError>
where
    L: EnvironmentLookup,
{
    let Some(raw) = raw else {
        return Ok(None);
    };
    let prefix = format!("model_list.{index}.model_info.access_windows");
    let mut windows = Vec::with_capacity(raw.len());
    for (position, window) in raw.into_iter().enumerate() {
        windows.push(window_config(
            &format!("{prefix}.{position}"),
            window,
            lookup,
        )?);
    }
    Ok(Some(windows))
}

fn window_config<L>(path: &str, window: RawWindow, lookup: &L) -> Result<WindowConfig, ConfigError>
where
    L: EnvironmentLookup,
{
    Ok(WindowConfig {
        start: resolved(window.start, &format!("{path}.start"), lookup)?,
        end: resolved(window.end, &format!("{path}.end"), lookup)?,
        timezone: resolved(window.timezone, &format!("{path}.timezone"), lookup)?,
        team_ids: window
            .team_ids
            .map(|ids| {
                ids.into_iter()
                    .enumerate()
                    .map(|(position, id)| {
                        resolve_string(id, &format!("{path}.team_ids.{position}"), lookup)
                    })
                    .collect::<Result<Vec<_>, ConfigError>>()
            })
            .transpose()?,
    })
}

fn resolved<L>(value: Option<String>, path: &str, lookup: &L) -> Result<Option<String>, ConfigError>
where
    L: EnvironmentLookup,
{
    value
        .map(|text| resolve_string(text, path, lookup))
        .transpose()
}
