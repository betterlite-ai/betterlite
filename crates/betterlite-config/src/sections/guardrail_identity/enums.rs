use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SupportedGuardrailIntegrations {
    #[serde(rename = "aporia")]
    Aporia,
    #[serde(rename = "bedrock")]
    Bedrock,
    #[serde(rename = "dynamoai")]
    Dynamoai,
    #[serde(rename = "guardrails_ai")]
    GuardrailsAi,
    #[serde(rename = "lakera")]
    Lakera,
    #[serde(rename = "lakera_v2")]
    LakeraV2,
    #[serde(rename = "presidio")]
    Presidio,
    #[serde(rename = "hide-secrets")]
    HideSecrets,
    #[serde(rename = "hiddenlayer")]
    Hiddenlayer,
    #[serde(rename = "aim")]
    Aim,
    #[serde(rename = "cato_networks")]
    CatoNetworks,
    #[serde(rename = "pangea")]
    Pangea,
    #[serde(rename = "crowdstrike_aidr")]
    CrowdstrikeAidr,
    #[serde(rename = "lasso")]
    Lasso,
    #[serde(rename = "pillar")]
    Pillar,
    #[serde(rename = "grayswan")]
    Grayswan,
    #[serde(rename = "panw_prisma_airs")]
    PanwPrismaAirs,
    #[serde(rename = "cisco_ai_defense")]
    CiscoAiDefense,
    #[serde(rename = "azure/prompt_shield")]
    AzurePromptShield,
    #[serde(rename = "azure/text_moderations")]
    AzureTextModerations,
    #[serde(rename = "model_armor")]
    ModelArmor,
    #[serde(rename = "openai_moderation")]
    OpenaiModeration,
    #[serde(rename = "noma")]
    Noma,
    #[serde(rename = "noma_v2")]
    NomaV2,
    #[serde(rename = "tool_permission")]
    ToolPermission,
    #[serde(rename = "zscaler_ai_guard")]
    ZscalerAiGuard,
    #[serde(rename = "javelin")]
    Javelin,
    #[serde(rename = "enkryptai")]
    Enkryptai,
    #[serde(rename = "ibm_guardrails")]
    IbmGuardrails,
    #[serde(rename = "litellm_content_filter")]
    LitellmContentFilter,
    #[serde(rename = "mcp_security")]
    McpSecurity,
    #[serde(rename = "onyx")]
    Onyx,
    #[serde(rename = "promptguard")]
    Promptguard,
    #[serde(rename = "xecguard")]
    Xecguard,
    #[serde(rename = "prompt_security")]
    PromptSecurity,
    #[serde(rename = "generic_guardrail_api")]
    GenericGuardrailApi,
    #[serde(rename = "qualifire")]
    Qualifire,
    #[serde(rename = "custom_code")]
    CustomCode,
    #[serde(rename = "ovalix")]
    Ovalix,
    #[serde(rename = "microsoft_purview")]
    MicrosoftPurview,
    #[serde(rename = "semantic_guard")]
    SemanticGuard,
    #[serde(rename = "mcp_end_user_permission")]
    McpEndUserPermission,
    #[serde(rename = "block_code_execution")]
    BlockCodeExecution,
    #[serde(rename = "akto")]
    Akto,
    #[serde(rename = "mcp_jwt_signer")]
    McpJwtSigner,
    #[serde(rename = "llm_as_a_judge")]
    LlmAsAJudge,
    #[serde(rename = "deepkeep")]
    Deepkeep,
    #[serde(rename = "qostodian_nexus")]
    QostodianNexus,
    #[serde(rename = "rubrik")]
    Rubrik,
    #[serde(rename = "vigil_guard")]
    VigilGuard,
    #[serde(rename = "repelloai")]
    Repelloai,
    #[serde(rename = "singulr")]
    Singulr,
    #[serde(rename = "headroom")]
    Headroom,
    #[serde(rename = "compresr")]
    Compresr,
    #[serde(rename = "typesafe")]
    Typesafe,
    #[serde(rename = "straiker")]
    Straiker,
    #[serde(rename = "alice")]
    Alice,
    #[serde(rename = "agent_365")]
    Agent365,
    #[serde(rename = "conduct")]
    Conduct,
    #[serde(rename = "decision_model")]
    DecisionModel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    #[serde(rename = "system")]
    System,
    #[serde(rename = "assistant")]
    Assistant,
    #[serde(rename = "user")]
    User,
}

#[must_use]
pub fn default_roles() -> Vec<Role> {
    let roles = [Role::System, Role::Assistant, Role::User];
    roles.to_vec()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PiiAction {
    #[serde(rename = "BLOCK")]
    Block,
    #[serde(rename = "MASK")]
    Mask,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum PiiEntityCategory {
    General,
    Finance,
    #[serde(rename = "USA")]
    Usa,
    #[serde(rename = "UK")]
    Uk,
    Spain,
    Italy,
    Poland,
    Singapore,
    Australia,
    India,
    Finland,
    Germany,
    Korea,
    Canada,
    Sweden,
    Thailand,
    Turkey,
    Nigeria,
    Philippines,
    #[serde(rename = "South Africa")]
    SouthAfrica,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PiiEntityType {
    CreditCard,
    Crypto,
    DateTime,
    EmailAddress,
    IbanCode,
    IpAddress,
    Nrp,
    Location,
    Person,
    PhoneNumber,
    MedicalLicense,
    Url,
    MacAddress,
    Uuid,
    UsBankNumber,
    UsDriverLicense,
    UsItin,
    UsPassport,
    UsSsn,
    UsMbi,
    UsNpi,
    UkNhs,
    UkNino,
    UkPassport,
    UkPostcode,
    UkVehicleRegistration,
    UkDrivingLicence,
    EsNif,
    EsNie,
    EsPassport,
    ItFiscalCode,
    ItDriverLicense,
    ItVatCode,
    ItPassport,
    ItIdentityCard,
    PlPesel,
    SgNricFin,
    SgUen,
    AuAbn,
    #[serde(rename = "AU_\x41\x43\x4e")]
    AuAcNumber,
    AuTfn,
    AuMedicare,
    InPan,
    InAadhaar,
    InVehicleRegistration,
    InVoter,
    InPassport,
    InGstin,
    FiPersonalIdentityCode,
    DeTaxId,
    DeTaxNumber,
    DeVatId,
    DePassport,
    DeIdCard,
    DeFuehrerschein,
    DeSocialSecurity,
    DeHealthInsurance,
    DeLanr,
    DeBsnr,
    DeKfz,
    DeHandelsregister,
    DePlz,
    KrRrn,
    KrFrn,
    KrPassport,
    KrDriverLicense,
    KrBrn,
    CaSin,
    SePersonnummer,
    SeOrganisationsnummer,
    ThTnin,
    TrNationalId,
    TrLicensePlate,
    #[serde(rename = "NG_\x4e\x49\x4e")]
    NgNationalId,
    NgVehicleRegistration,
    PhTin,
    PhUmid,
    PhPassport,
    ZaIdNumber,
}

include!("pii_map.rs");
