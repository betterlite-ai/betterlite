use super::presence::Field;
use super::presence::optional;
use serde::{Deserialize, Serialize};

use super::SapModelError;
use super::presence::{Given, NonEmpty};

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
pub enum MaskingEntity {
    #[serde(rename = "profile-person")]
    Person,
    #[serde(rename = "profile-org")]
    Org,
    #[serde(rename = "profile-university")]
    University,
    #[serde(rename = "profile-location")]
    Location,
    #[serde(rename = "profile-email")]
    Email,
    #[serde(rename = "profile-phone")]
    Phone,
    #[serde(rename = "profile-address")]
    Address,
    #[serde(rename = "profile-sapids-internal")]
    SapIdsInternal,
    #[serde(rename = "profile-sapids-public")]
    SapIdsPublic,
    #[serde(rename = "profile-url")]
    Url,
    #[serde(rename = "profile-username-password")]
    UsernamePassword,
    #[serde(rename = "profile-nationalid")]
    NationalId,
    #[serde(rename = "profile-iban")]
    Iban,
    #[serde(rename = "profile-ssn")]
    Ssn,
    #[serde(rename = "profile-credit-card-number")]
    CreditCardNumber,
    #[serde(rename = "profile-passport")]
    Passport,
    #[serde(rename = "profile-driverlicense")]
    DrivingLicense,
    #[serde(rename = "profile-nationality")]
    Nationality,
    #[serde(rename = "profile-religious-group")]
    ReligiousGroup,
    #[serde(rename = "profile-political-group")]
    PoliticalGroup,
    #[serde(rename = "profile-pronouns-gender")]
    PronounsGender,
    #[serde(rename = "profile-gender")]
    Gender,
    #[serde(rename = "profile-sexual-orientation")]
    SexualOrientation,
    #[serde(rename = "profile-trade-union")]
    TradeUnion,
    #[serde(rename = "profile-sensitive-data")]
    SensitiveData,
    #[serde(rename = "profile-ethnicity")]
    Ethnicity,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ConstantMethod {
    Constant,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FabricatedMethod {
    FabricatedData,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DPIMethodConstant {
    #[serde(default, skip_serializing_if = "Given::is_missing")]
    pub method: Given<ConstantMethod>,
    pub value: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DPIMethodFabricatedData {
    #[serde(default, skip_serializing_if = "Given::is_missing")]
    pub method: Given<FabricatedMethod>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum ReplacementStrategy {
    Constant(DPIMethodConstant),
    Fabricated(DPIMethodFabricatedData),
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DPICustomEntity {
    pub regex: String,
    pub replacement_strategy: DPIMethodConstant,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DPIStandardEntity {
    #[serde(rename = "type")]
    pub type_: MaskingEntity,
    #[serde(default, with = "optional", skip_serializing_if = "Option::is_none")]
    pub replacement_strategy: Field<ReplacementStrategy>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum MaskingEntityChoice {
    Standard(DPIStandardEntity),
    Custom(DPICustomEntity),
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MaskGroundingInput {
    #[serde(default, skip_serializing_if = "Given::is_missing")]
    pub enabled: Given<bool>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PrivacyKind {
    SapDataPrivacyIntegration,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MaskingMethod {
    Anonymization,
    Pseudonymization,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MaskingProviderConfig {
    #[serde(rename = "type", default, skip_serializing_if = "Given::is_missing")]
    pub type_: Given<PrivacyKind>,
    pub method: MaskingMethod,
    pub entities: Vec<MaskingEntityChoice>,
    #[serde(default, with = "optional", skip_serializing_if = "Option::is_none")]
    pub allowlist: Field<Vec<String>>,
    #[serde(default, with = "optional", skip_serializing_if = "Option::is_none")]
    pub mask_grounding_input: Field<MaskGroundingInput>,
}

#[derive(Debug, Deserialize)]
struct MaskingModuleConfigRaw {
    #[serde(default, with = "optional")]
    providers: Field<NonEmpty<MaskingProviderConfig>>,
    #[serde(default, with = "optional")]
    masking_providers: Field<NonEmpty<MaskingProviderConfig>>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(try_from = "MaskingModuleConfigRaw")]
pub struct MaskingModuleConfig {
    #[serde(default, with = "optional", skip_serializing_if = "Option::is_none")]
    pub providers: Field<NonEmpty<MaskingProviderConfig>>,
    #[serde(default, with = "optional", skip_serializing_if = "Option::is_none")]
    pub masking_providers: Field<NonEmpty<MaskingProviderConfig>>,
}

impl TryFrom<MaskingModuleConfigRaw> for MaskingModuleConfig {
    type Error = SapModelError;

    fn try_from(raw: MaskingModuleConfigRaw) -> Result<Self, Self::Error> {
        let has_providers = matches!(&raw.providers, Some(Some(_)));
        let has_masking_providers = matches!(&raw.masking_providers, Some(Some(_)));
        if !has_providers && !has_masking_providers {
            return Err(SapModelError::Rejected {
                message: String::from(
                    "For SAP Masking Module Config you must provide 'providers'.",
                ),
            });
        }
        if has_providers && has_masking_providers {
            return Err(SapModelError::Rejected {
                message: String::from(
                    "For SAP Masking Module Config you must set exactly one of: 'providers' or 'masking_providers', not both.",
                ),
            });
        }
        if has_masking_providers {
            tracing::warn!(
                "The 'masking_providers' parameter is deprecated and will be removed on Sept 15, 2026. Use 'providers' instead."
            );
        }
        Ok(Self {
            providers: raw.providers,
            masking_providers: raw.masking_providers,
        })
    }
}
