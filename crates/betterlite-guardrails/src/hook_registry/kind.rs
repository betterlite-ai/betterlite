use std::sync::Arc;

use betterlite_config::sections::guardrail_params::params::LitellmParams;

use super::kinds::{AliceArgs, CatoNetworksArgs, DynamoAiArgs, EnkryptaiArgs};
use crate::content_filter_init::args::ContentFilterArgs;
use crate::hook_initializers::agent_365::Agent365Args;
use crate::hook_initializers::azure::AzureArgs;
use crate::hook_initializers::ibm::IbmArgs;
use crate::hook_initializers::model_armor::ModelArmorArgs;
use crate::hook_initializers::openai::OpenAiArgs;
use crate::hook_initializers::purview::PurviewArgs;
use crate::mcp_jwt_signer::JwtSigner;
use crate::promptguard::init::PromptGuardArgs;
use crate::qohash::init::QohashArgs;
use crate::qualifire::init::QualifireArgs;
use crate::semantic_guard_hook::init::SemanticGuardArgs;
use crate::singulr::init::SingulrArgs;
use crate::third_party_init_a::aim::AimArgs;
use crate::third_party_init_a::akto::AktoArgs;
use crate::third_party_init_a::aporia::AporiaArgs;
use crate::third_party_init_a::cisco::CiscoArgs;
use crate::third_party_init_a::conduct::ConductArgs;
use crate::third_party_init_a::crowdstrike::CrowdStrikeArgs;
use crate::third_party_init_a::deepkeep::DeepKeepArgs;
use crate::third_party_init_a::generic_api::GenericApiArgs;
use crate::third_party_init_a::grayswan::GraySwanArgs;
use crate::third_party_init_a::guardrails_ai::GuardrailsAiArgs;
use crate::third_party_init_a::headroom::HeadroomArgs;
use crate::third_party_init_a::lasso::LassoFolderArgs;
use crate::third_party_init_b::panw::PanwFolderArgs;
use crate::third_party_init_b::pillar::PillarArgs;
use crate::third_party_init_b::rubrik::RubrikArgs;
use crate::third_party_init_b::straiker::StraikerArgs;
use crate::third_party_init_b::typesafe::TypeSafeArgs;
use crate::third_party_init_b::vigil_guard::VigilGuardArgs;
use crate::third_party_init_b::xecguard::XecGuardArgs;
use crate::zscaler_ai_guard::init::ZscalerArgs;

// One variant per wire name, holding what that kind's initializer returned
pub enum HookKind {
    Aim(Box<AimArgs>),
    Akto(Box<AktoArgs>),
    Aporia(Box<AporiaArgs>),
    CiscoAiDefense(Box<CiscoArgs>),
    Conduct(Box<ConductArgs>),
    CrowdstrikeAidr(Box<CrowdStrikeArgs>),
    Deepkeep(Box<DeepKeepArgs>),
    GenericGuardrailApi(Box<GenericApiArgs>),
    Grayswan(Box<GraySwanArgs>),
    GuardrailsAi(Box<GuardrailsAiArgs>),
    Headroom(Box<HeadroomArgs>),
    Lasso(Box<LassoFolderArgs>),
    PanwPrismaAirs(Box<PanwFolderArgs>),
    Pillar(Box<PillarArgs>),
    Rubrik(Box<RubrikArgs>),
    Straiker(Box<StraikerArgs>),
    Typesafe(Box<TypeSafeArgs>),
    VigilGuard(Box<VigilGuardArgs>),
    Xecguard(Box<XecGuardArgs>),
    Agent365(Box<Agent365Args>),
    AzurePromptShield(Box<AzureArgs>),
    AzureTextModerations(Box<AzureArgs>),
    IbmGuardrails(Box<IbmArgs>),
    MicrosoftPurview(Box<PurviewArgs>),
    ModelArmor(Box<ModelArmorArgs>),
    OpenaiModeration(Box<OpenAiArgs>),
    LitellmContentFilter(Box<ContentFilterArgs>),
    McpJwtSigner(Arc<JwtSigner>),
    Promptguard(Box<PromptGuardArgs>),
    QostodianNexus(Box<QohashArgs>),
    Qualifire(Box<QualifireArgs>),
    SemanticGuard(Box<SemanticGuardArgs>),
    Singulr(Box<SingulrArgs>),
    ZscalerAiGuard(Box<ZscalerArgs>),
    // the class tasks build these from the params, so the spec keeps the params
    BlockCodeExecution(Box<LitellmParams>),
    Compresr(Box<LitellmParams>),
    CustomCode(Box<LitellmParams>),
    Hiddenlayer(Box<LitellmParams>),
    Javelin(Box<LitellmParams>),
    LlmAsAJudge(Box<LitellmParams>),
    Noma(Box<LitellmParams>),
    NomaV2(Box<LitellmParams>),
    Onyx(Box<LitellmParams>),
    Ovalix(Box<LitellmParams>),
    Pangea(Box<LitellmParams>),
    Repelloai(Box<LitellmParams>),
    Alice(Box<AliceArgs>),
    CatoNetworks(Box<CatoNetworksArgs>),
    Dynamoai(Box<DynamoAiArgs>),
    Enkryptai(Box<EnkryptaiArgs>),
}
