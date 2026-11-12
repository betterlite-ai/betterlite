use serde::{Deserialize, Serialize};

use super::BedrockChecksConfigModel;

#[derive(Debug, thiserror::Error)]
pub enum BedrockChecksError {
    #[error(
        "Bedrock 'checks' must enable at least one of: contentFilter, promptAttack, sensitiveInformation."
    )]
    NoCheckEnabled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum BedrockContentFilterCategory {
    Violence,
    Hate,
    Sexual,
    Misconduct,
    Insults,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum BedrockPromptAttackCategory {
    Jailbreak,
    PromptInjection,
    PromptLeakage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum BedrockSensitiveInfoEntity {
    Address,
    Age,
    AwsAccessKey,
    AwsSecretKey,
    CaHealthNumber,
    CaSocialInsuranceNumber,
    CreditDebitCardCvv,
    CreditDebitCardExpiry,
    CreditDebitCardNumber,
    DriverId,
    Email,
    InternationalBankAccountNumber,
    IpAddress,
    LicensePlate,
    MacAddress,
    Name,
    Password,
    Phone,
    Pin,
    SwiftCode,
    UkNationalHealthServiceNumber,
    UkNationalInsuranceNumber,
    #[serde(rename = "UK_UNIQUE_TAXPAYER_REFERENCE_NUMBER")]
    UkUniqueTaxpayerReference,
    Url,
    Username,
    UsBankAccountNumber,
    UsBankRoutingNumber,
    #[serde(rename = "US_INDIVIDUAL_TAX_IDENTIFICATION_NUMBER")]
    UsIndividualTaxId,
    UsPassportNumber,
    UsSocialSecurityNumber,
    VehicleIdentificationNumber,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BedrockContentFilterItem {
    pub category: BedrockContentFilterCategory,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BedrockContentFilterModel {
    pub categories: Vec<BedrockContentFilterItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BedrockPromptAttackItem {
    pub category: BedrockPromptAttackCategory,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BedrockPromptAttackModel {
    pub categories: Vec<BedrockPromptAttackItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BedrockSensitiveInfoItem {
    #[serde(rename = "type")]
    pub entity_type: BedrockSensitiveInfoEntity,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BedrockSensitiveInfoModel {
    pub entities: Vec<BedrockSensitiveInfoItem>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct BedrockChecksConfigWire {
    #[serde(rename = "contentFilter", default)]
    pub(crate) content_filter: Option<BedrockContentFilterModel>,
    #[serde(rename = "promptAttack", default)]
    pub(crate) prompt_attack: Option<BedrockPromptAttackModel>,
    #[serde(rename = "sensitiveInformation", default)]
    pub(crate) sensitive_information: Option<BedrockSensitiveInfoModel>,
}

impl TryFrom<BedrockChecksConfigWire> for BedrockChecksConfigModel {
    type Error = BedrockChecksError;

    fn try_from(wire: BedrockChecksConfigWire) -> Result<Self, Self::Error> {
        let model = Self {
            content_filter: wire.content_filter,
            prompt_attack: wire.prompt_attack,
            sensitive_information: wire.sensitive_information,
        };
        model.require_at_least_one_check()?;
        Ok(model)
    }
}
