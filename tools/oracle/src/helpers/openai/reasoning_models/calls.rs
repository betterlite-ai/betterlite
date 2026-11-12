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
    const fn parts(&self) -> (Mode, &Probes) {
        match self {
            Self::Gpt5SearchAllowlist(probes) => (Mode::SearchMapping, probes),
            Self::Gpt5ChatExclusion(probes) => (Mode::Policy, probes),
            Self::Gpt6FamilySelection(probes) => (Mode::Gpt6, probes),
            Self::DictEffortNormalization(probes)
            | Self::XhighOptIn(probes)
            | Self::LowMinimalOptOut(probes)
            | Self::TemperatureWhileThinking(probes)
            | Self::SamplingWithNone(probes) => (Mode::GptMapping, probes),
            Self::CatalogBeforeDefaultKey(probes) => (Mode::Effort, probes),
            Self::O1DeveloperRoles(probes) => (Mode::Messages, probes),
            Self::AudioTokenParameter(probes) => (Mode::Audio, probes),
            Self::Gpt5RegularAllowlist(probes) => (Mode::GptSupported, probes),
            Self::VersionBoundaries(probes) => (Mode::Version, probes),
            Self::OSeriesCapabilityFallback(probes) => (Mode::OSeriesSupported, probes),
            Self::OSeriesTemperatureLimit(probes) => (Mode::OSeriesMapping, probes),
        }
    }
}

pub(super) async fn run(call: &Call) -> HelperResult {
    let (mode, probes) = call.parts();
    let mut results = Vec::with_capacity(probes.probes.len());
    for probe in &probes.probes {
        results.push(outcome(probe, mode).await);
    }
    Ok(Value::Array(results))
}

const fn drop_params(probe: &Probe) -> DropParams {
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
    if let (Some(supported_list), Value::Object(fields)) = (supported, &mut value) {
        fields.insert(String::from("supported"), json!(supported_list));
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
    let catalog = fixture.catalog;
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
    if let Value::Object(fields) = &mut value {
        fields.insert(
            String::from("policy"),
            json!(select_policy(&probe.model, &fixture.catalog)),
        );
    }
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
