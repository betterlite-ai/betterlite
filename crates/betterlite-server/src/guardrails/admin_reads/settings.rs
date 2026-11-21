use std::collections::BTreeMap;
use std::sync::Arc;

use axum::Json;
use axum::extract::{Extension, State};
use betterlite_config::sections::content_filter_catalog::categories::available_content_categories;
use betterlite_config::sections::content_filter_catalog::patterns::{
    PatternCatalogError, PatternMetadata, get_pattern_metadata,
};
use betterlite_config::sections::content_filter_catalog::roots::{
    ContentFilterRootsError, data_roots,
};
use betterlite_config::sections::guardrail_identity::enums::{
    PII_ENTITY_CATEGORIES_MAP, PiiAction, PiiEntityType,
};
use betterlite_config::sections::guardrail_params::error::GuardrailParamsError;
use betterlite_config::sections::guardrail_params::records::{
    GuardrailEventHooks, PiiEntityCategoryMap,
};
use betterlite_config::sections::guardrail_params::routes::GuardrailUiAddSettings;
use betterlite_guardrails::guardrail_registry::rows::GuardrailRows;
use betterlite_openai::decisions::providers::supported_decisions_providers;
use betterlite_pyhost::json_values::JsonObject;

use super::caller::authenticated;
use super::error::ReadError;
use super::projection::opaque_params;
use super::provider_modes::PROVIDER_MODES;
use super::state::ReadState;
use crate::auth::auth_identity::AuthContext;

const PII_ACTIONS: [PiiAction; 2] = [PiiAction::Block, PiiAction::Mask];
const CONTENT_FILTER_ACTIONS: [&str; 2] = ["BLOCK", "MASK"];
const SUPPORTED_MODES: [GuardrailEventHooks; 8] = [
    GuardrailEventHooks::PreCall,
    GuardrailEventHooks::PostCall,
    GuardrailEventHooks::DuringCall,
    GuardrailEventHooks::LoggingOnly,
    GuardrailEventHooks::PreMcpCall,
    GuardrailEventHooks::DuringMcpCall,
    GuardrailEventHooks::PostMcpCall,
    GuardrailEventHooks::RealtimeInputTranscription,
];

// PiiEntityType in declaration order: LiteLLM lists the enum members, and serde keeps that order here
const PII_ENTITY_TYPES: [PiiEntityType; 79] = [
    PiiEntityType::CreditCard,
    PiiEntityType::Crypto,
    PiiEntityType::DateTime,
    PiiEntityType::EmailAddress,
    PiiEntityType::IbanCode,
    PiiEntityType::IpAddress,
    PiiEntityType::Nrp,
    PiiEntityType::Location,
    PiiEntityType::Person,
    PiiEntityType::PhoneNumber,
    PiiEntityType::MedicalLicense,
    PiiEntityType::Url,
    PiiEntityType::MacAddress,
    PiiEntityType::Uuid,
    PiiEntityType::UsBankNumber,
    PiiEntityType::UsDriverLicense,
    PiiEntityType::UsItin,
    PiiEntityType::UsPassport,
    PiiEntityType::UsSsn,
    PiiEntityType::UsMbi,
    PiiEntityType::UsNpi,
    PiiEntityType::UkNhs,
    PiiEntityType::UkNino,
    PiiEntityType::UkPassport,
    PiiEntityType::UkPostcode,
    PiiEntityType::UkVehicleRegistration,
    PiiEntityType::UkDrivingLicence,
    PiiEntityType::EsNif,
    PiiEntityType::EsNie,
    PiiEntityType::EsPassport,
    PiiEntityType::ItFiscalCode,
    PiiEntityType::ItDriverLicense,
    PiiEntityType::ItVatCode,
    PiiEntityType::ItPassport,
    PiiEntityType::ItIdentityCard,
    PiiEntityType::PlPesel,
    PiiEntityType::SgNricFin,
    PiiEntityType::SgUen,
    PiiEntityType::AuAbn,
    PiiEntityType::AuAcNumber,
    PiiEntityType::AuTfn,
    PiiEntityType::AuMedicare,
    PiiEntityType::InPan,
    PiiEntityType::InAadhaar,
    PiiEntityType::InVehicleRegistration,
    PiiEntityType::InVoter,
    PiiEntityType::InPassport,
    PiiEntityType::InGstin,
    PiiEntityType::FiPersonalIdentityCode,
    PiiEntityType::DeTaxId,
    PiiEntityType::DeTaxNumber,
    PiiEntityType::DeVatId,
    PiiEntityType::DePassport,
    PiiEntityType::DeIdCard,
    PiiEntityType::DeFuehrerschein,
    PiiEntityType::DeSocialSecurity,
    PiiEntityType::DeHealthInsurance,
    PiiEntityType::DeLanr,
    PiiEntityType::DeBsnr,
    PiiEntityType::DeKfz,
    PiiEntityType::DeHandelsregister,
    PiiEntityType::DePlz,
    PiiEntityType::KrRrn,
    PiiEntityType::KrFrn,
    PiiEntityType::KrPassport,
    PiiEntityType::KrDriverLicense,
    PiiEntityType::KrBrn,
    PiiEntityType::CaSin,
    PiiEntityType::SePersonnummer,
    PiiEntityType::SeOrganisationsnummer,
    PiiEntityType::ThTnin,
    PiiEntityType::TrNationalId,
    PiiEntityType::TrLicensePlate,
    PiiEntityType::NgNationalId,
    PiiEntityType::NgVehicleRegistration,
    PiiEntityType::PhTin,
    PiiEntityType::PhUmid,
    PiiEntityType::PhPassport,
    PiiEntityType::ZaIdNumber,
];

#[derive(Debug, thiserror::Error)]
enum SettingsError {
    #[error("{0}")]
    Patterns(#[from] PatternCatalogError),
    #[error("{0}")]
    Roots(#[from] ContentFilterRootsError),
    #[error(transparent)]
    Params(#[from] GuardrailParamsError),
    #[error(transparent)]
    Encode(#[from] serde_json::Error),
}

impl From<SettingsError> for ReadError {
    fn from(error: SettingsError) -> Self {
        Self::Internal {
            detail: error.to_string(),
        }
    }
}

pub(super) async fn guardrail_ui_settings<R>(
    caller: Option<Extension<AuthContext>>,
    State(_state): State<Arc<ReadState<R>>>,
) -> Result<Json<GuardrailUiAddSettings>, ReadError>
where
    R: GuardrailRows,
{
    authenticated(caller)?;
    let settings = settings_body()?;
    Ok(Json(settings))
}

fn settings_body() -> Result<GuardrailUiAddSettings, SettingsError> {
    let patterns = get_pattern_metadata()?;
    let content_filter_settings = content_filter_settings(&patterns)?;
    Ok(GuardrailUiAddSettings {
        supported_entities: serde_json::from_slice(&serde_json::to_vec(&PII_ENTITY_TYPES)?)?,
        supported_actions: serde_json::from_slice(&serde_json::to_vec(&PII_ACTIONS)?)?,
        supported_modes: serde_json::from_slice(&serde_json::to_vec(&SUPPORTED_MODES)?)?,
        supported_modes_by_provider: provider_modes(),
        pii_entity_categories: pii_entity_categories()?,
        content_filter_settings: Some(opaque_params(&content_filter_settings)?),
        decision_model_providers: supported_decisions_providers()
            .into_iter()
            .map(str::to_owned)
            .collect(),
    })
}

fn content_filter_settings(patterns: &[PatternMetadata]) -> Result<JsonObject, SettingsError> {
    let [bundled, legacy] = data_roots()?;
    let categories = available_content_categories(&[bundled.as_path(), legacy.as_path()]);
    let mut settings = JsonObject::new();
    settings.insert(
        "prebuilt_patterns".to_owned(),
        serde_json::from_slice(&serde_json::to_vec(patterns)?)?,
    );
    settings.insert(
        "pattern_categories".to_owned(),
        serde_json::from_slice(&serde_json::to_vec(&pattern_categories(patterns))?)?,
    );
    settings.insert(
        "supported_actions".to_owned(),
        serde_json::from_slice(&serde_json::to_vec(&CONTENT_FILTER_ACTIONS)?)?,
    );
    settings.insert(
        "content_categories".to_owned(),
        serde_json::from_slice(&serde_json::to_vec(&categories)?)?,
    );
    Ok(settings)
}

// PATTERN_CATEGORIES keeps the order its first appearance in patterns.json gives it
fn pattern_categories(patterns: &[PatternMetadata]) -> Vec<String> {
    let mut categories: Vec<String> = Vec::new();
    for pattern in patterns {
        if !categories.contains(&pattern.category) {
            categories.push(pattern.category.clone());
        }
    }
    categories
}

fn pii_entity_categories() -> Result<Vec<PiiEntityCategoryMap>, serde_json::Error> {
    let mut maps = Vec::with_capacity(PII_ENTITY_CATEGORIES_MAP.len());
    for (category, entities) in &PII_ENTITY_CATEGORIES_MAP {
        maps.push(PiiEntityCategoryMap {
            category: serde_json::from_slice(&serde_json::to_vec(category)?)?,
            entities: serde_json::from_slice(&serde_json::to_vec(entities)?)?,
        });
    }
    Ok(maps)
}

fn provider_modes() -> BTreeMap<String, Vec<String>> {
    PROVIDER_MODES
        .iter()
        .map(|(provider, hooks)| {
            (
                (*provider).to_owned(),
                hooks.iter().map(|hook| (*hook).to_owned()).collect(),
            )
        })
        .collect()
}
