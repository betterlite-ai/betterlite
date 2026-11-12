use betterlite_openai::reasoning_models::{
    DropParams, audio, effort_resolves_to_none, gpt_5, gpt_series_version, is_model_gpt_5_2_model,
    is_model_gpt_5_4_plus_model, is_model_gpt_5_6_plus_model, is_model_gpt_6_plus_model, o_series,
    select_policy,
};
use serde_json::{Value, json};

use super::fixture::{Fixture, Resolver};
use super::{Call, Probe, Probes};
use crate::helpers::HelperResult;

#[derive(Clone, Copy)]
enum Mode {
    GptMapping,
    SearchMapping,
    OSeriesMapping,
    GptSupported,
    OSeriesSupported,
    Audio,
    Policy,
    Gpt6,
    Version,
    Effort,
    Messages,
}

impl Call {
    fn mode(&self) -> Mode {
        match self {
            Self::Gpt5SearchAllowlist(_) => Mode::SearchMapping,
            Self::Gpt5ChatExclusion(_) => Mode::Policy,
            Self::Gpt6FamilySelection(_) => Mode::Gpt6,
            Self::DictEffortNormalization(_)
            | Self::XhighOptIn(_)
            | Self::LowMinimalOptOut(_)
            | Self::TemperatureWhileThinking(_)
            | Self::SamplingWithNone(_) => Mode::GptMapping,
            Self::CatalogBeforeDefaultKey(_) => Mode::Effort,
            Self::O1DeveloperRoles(_) => Mode::Messages,
            Self::AudioTokenParameter(_) => Mode::Audio,
            Self::Gpt5RegularAllowlist(_) => Mode::GptSupported,
            Self::VersionBoundaries(_) => Mode::Version,
            Self::OSeriesCapabilityFallback(_) => Mode::OSeriesSupported,
            Self::OSeriesTemperatureLimit(_) => Mode::OSeriesMapping,
        }
    }

    fn probes(&self) -> &Probes {
        match self {
            Self::Gpt5SearchAllowlist(probes)
            | Self::Gpt5ChatExclusion(probes)
            | Self::Gpt6FamilySelection(probes)
            | Self::DictEffortNormalization(probes)
            | Self::XhighOptIn(probes)
            | Self::LowMinimalOptOut(probes)
            | Self::CatalogBeforeDefaultKey(probes)
            | Self::TemperatureWhileThinking(probes)
            | Self::SamplingWithNone(probes)
            | Self::O1DeveloperRoles(probes)
            | Self::AudioTokenParameter(probes)
            | Self::Gpt5RegularAllowlist(probes)
            | Self::VersionBoundaries(probes)
            | Self::OSeriesCapabilityFallback(probes)
            | Self::OSeriesTemperatureLimit(probes) => probes,
        }
    }
}

pub(super) async fn run(call: &Call) -> HelperResult {
    let mode = call.mode();
    let mut results = Vec::with_capacity(call.probes().probes.len());
    for probe in &call.probes().probes {
        results.push(outcome(probe, mode).await);
    }
    Ok(Value::Array(results))
}

fn drop_params(probe: &Probe) -> DropParams {
    DropParams {
        global: probe.global_drop_params,
        request: probe.drop_params,
    }
}

async fn outcome(probe: &Probe, mode: Mode) -> Value {
    match mode {
        Mode::GptMapping => gpt_mapping(probe, false).await,
        Mode::SearchMapping => gpt_mapping(probe, true).await,
        Mode::OSeriesMapping => o_series_mapping(probe).await,
        Mode::GptSupported => {
            let supported = gpt_supported(probe).await;
            json!({"supported": supported})
        }
        Mode::OSeriesSupported => {
            let supported = o_series_supported(probe).await;
            json!({"supported": supported})
        }
        Mode::Audio => audio_value(probe),
        Mode::Policy => policy(probe),
        Mode::Gpt6 => gpt6(probe),
        Mode::Version => version_value(probe),
        Mode::Effort => effort(probe).await,
        Mode::Messages => messages(probe).await,
    }
}

async fn gpt_mapping(probe: &Probe, search: bool) -> Value {
    let mut fixture = Fixture::new(probe);
    let catalog = fixture.catalog.clone();
    let supported = if search {
        Some(gpt_5::supported_params(&mut fixture, &probe.model, &catalog).await)
    } else {
        None
    };
    let mut non_default = probe.requested.clone();
    let mut optional = probe.existing.clone();
    let mapped = gpt_5::map_params(
        &mut fixture,
        &mut gpt_5::Mapping {
            model: &probe.model,
            non_default_params: &mut non_default,
            optional_params: &mut optional,
            drop_params: drop_params(probe),
            catalog: &catalog,
        },
    )
    .await;
    let mut value = match mapped {
        Ok(()) => json!({"non_default_params": non_default, "optional_params": optional}),
        Err(error) => json!({"error": error.status_code()}),
    };
    if let Some(supported) = supported {
        value["supported"] = json!(supported);
    }
    value
}

async fn o_series_mapping(probe: &Probe) -> Value {
    let mut fixture = Fixture::new(probe);
    let catalog = fixture.catalog.clone();
    let mut non_default = probe.requested.clone();
    let mut optional = probe.existing.clone();
    match o_series::map_params(
        &mut fixture,
        &mut Resolver(probe.resolved.clone()),
        &mut o_series::Mapping {
            model: &probe.model,
            non_default_params: &mut non_default,
            optional_params: &mut optional,
            drop_params: drop_params(probe),
            catalog: &catalog,
        },
    )
    .await
    {
        Ok(()) => json!({"non_default_params": non_default, "optional_params": optional}),
        Err(error) => json!({"error": error.status_code()}),
    }
}

async fn gpt_supported(probe: &Probe) -> Vec<&'static str> {
    let mut fixture = Fixture::new(probe);
    let catalog = fixture.catalog.clone();
    gpt_5::supported_params(&mut fixture, &probe.model, &catalog).await
}

async fn o_series_supported(probe: &Probe) -> Vec<&'static str> {
    let mut fixture = Fixture::new(probe);
    let catalog = fixture.catalog.clone();
    o_series::supported_params(
        &mut fixture,
        &mut Resolver(probe.resolved.clone()),
        &probe.model,
        &catalog,
    )
    .await
}

fn audio_value(probe: &Probe) -> Value {
    let fixture = Fixture::new(probe);
    let catalog = fixture.catalog.clone();
    let supported = audio::supported_params(&probe.model, &catalog);
    let mut non_default = probe.requested.clone();
    let mut optional = probe.existing.clone();
    audio::map_params(&probe.model, &mut non_default, &mut optional, &catalog);
    json!({
        "supported": supported,
        "non_default_params": non_default,
        "optional_params": optional
    })
}

fn policy(probe: &Probe) -> Value {
    let fixture = Fixture::new(probe);
    let policy = select_policy(&probe.model, &fixture.catalog);
    json!({"policy": policy})
}

fn gpt6(probe: &Probe) -> Value {
    let fixture = Fixture::new(probe);
    let mut value = version_value(probe);
    value["policy"] = json!(select_policy(&probe.model, &fixture.catalog));
    value
}

fn version_value(probe: &Probe) -> Value {
    json!({
        "version": gpt_series_version(&probe.model),
        "gpt_5_4_plus": is_model_gpt_5_4_plus_model(&probe.model),
        "gpt_5_6_plus": is_model_gpt_5_6_plus_model(&probe.model),
        "gpt_6_plus": is_model_gpt_6_plus_model(&probe.model),
        "gpt_5_2": is_model_gpt_5_2_model(&probe.model),
    })
}

async fn effort(probe: &Probe) -> Value {
    let mut fixture = Fixture::new(probe);
    let resolves = effort_resolves_to_none(
        &mut fixture,
        &probe.model,
        probe.effective_effort.as_deref(),
    )
    .await;
    json!({"resolves_to_none": resolves})
}

async fn messages(probe: &Probe) -> Value {
    let mut fixture = Fixture::new(probe);
    let sync = o_series::transform_messages(&mut fixture, &probe.model, &probe.messages).await;
    let asynchronous =
        o_series::transform_messages(&mut fixture, &probe.model, &probe.messages).await;
    json!({"sync": sync, "async": asynchronous})
}
