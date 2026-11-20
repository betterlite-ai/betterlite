#![cfg(test)]

use std::collections::BTreeMap;
use std::io::{self, Write};
use std::sync::{Arc, Mutex};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::decode::decode_litellm_params;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks::{
    self, DuringCall, DuringMcpCall, LoggingOnly, PostCall, PostMcpCall, PreCall, PreMcpCall,
    RealtimeInputTranscription,
};
use betterlite_http::proxy::base_builder;
use reqwest::Client;
use reqwest::header::{HeaderMap, HeaderValue};
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use tracing::subscriber::set_default;
use tracing_subscriber::fmt;

use super::error::HookRegistryError;
use super::hooks::supported_event_hooks;
use super::kind::HookKind;
use super::kinds::build_hook_spec;
use super::ports::HookPorts;
use super::spec::HookSpec;
use super::wire;
use crate::guardrail_registry::build::GuardrailFactories;
use crate::guardrail_registry::callback::CallbackSpec;
use crate::guardrail_registry::error::RegistryError;
use crate::guardrail_startup::from_config::skips_startup;
use crate::hook_initializers::azure::AzureArgs;
use crate::hook_initializers::secret_lookup::SecretLookup;
use crate::mcp_jwt_signer_init::JwtSignerSlot;
use crate::tool_policy::{RequestToolNames, ToolPolicyLookup};

fn params_from(json: &str) -> LitellmParams {
    decode_litellm_params(json.as_bytes()).unwrap()
}

fn mode_of(kind: &str) -> &'static str {
    if kind == wire::MCP_JWT_SIGNER {
        "pre_mcp_call"
    } else {
        "pre_call"
    }
}

// every extra the third-party initializers require, plus the fields the ported args copy
fn test_params(kind: &str) -> LitellmParams {
    let mode = mode_of(kind);
    let body = format!(
        r#"{{"guardrail":"{kind}","mode":"{mode}","api_key":"test-key","api_base":"https://guard.example/v1","guard_name":"my-guard","tenant_id":"tenant","client_id":"client","client_secret":"secret","auth_token":"token","base_url":"https://ibm.example/v1","detector_id":"detector","project_id":"project","default_on":true,"timeout":7.5}}"#
    );
    params_from(&body)
}

struct NoSecrets;

impl SecretLookup for NoSecrets {
    fn get_secret_str(&self, _name: &str) -> Option<SecretString> {
        None
    }
}

fn test_ports() -> HookPorts {
    HookPorts {
        secrets: Arc::new(NoSecrets),
        environment: Arc::new(BTreeMap::<String, String>::new()),
        router_present: true,
        slot: Arc::new(JwtSignerSlot::new()),
        registered: Arc::new(Mutex::new(Vec::new())),
        client: Client::default(),
    }
}

fn exposed(secret: Option<&SecretString>) -> Option<&str> {
    secret.map(SecretString::expose_secret)
}

fn build_ok(kind: &str) -> HookSpec {
    let params = test_params(kind);
    build_hook_spec(kind, &params, "test-guard", &test_ports())
        .expect("wire name is built")
        .expect("initializer succeeds")
}

fn build_error_on(kind: &str, params: &LitellmParams) -> HookRegistryError {
    build_hook_spec(kind, params, "test-guard", &test_ports())
        .expect("wire name is built")
        .err()
        .expect("build fails")
}

fn build_error(kind: &str) -> HookRegistryError {
    let params = test_params(kind);
    build_error_on(kind, &params)
}

struct NoPolicies;

impl ToolPolicyLookup for NoPolicies {
    fn is_initialized(&self) -> bool {
        false
    }

    fn effective_policies(
        &self,
        _tool_names: &[String],
        _key_permission_id: Option<&str>,
        _team_permission_id: Option<&str>,
    ) -> BTreeMap<String, String> {
        BTreeMap::new()
    }

    fn output_policy(&self, _tool_name: &str) -> String {
        String::new()
    }
}

struct NoNames;

impl RequestToolNames for NoNames {
    fn names(&self, _route: &str, _request_data: &BTreeMap<String, GuardrailValue>) -> Vec<String> {
        Vec::new()
    }
}

fn factories() -> GuardrailFactories {
    GuardrailFactories::new(Arc::new(NoPolicies), Arc::new(NoNames))
}

// The supported event hooks of every wire name, written out as each class declares them
#[rstest]
#[case::hooks_agent_365(wire::AGENT_365, &[PreMcpCall])]
#[case::hooks_aim(wire::AIM, &[PreCall, DuringCall, PostCall])]
#[case::hooks_akto(wire::AKTO, &[PreCall, PostCall])]
#[case::hooks_alice(wire::ALICE, &[PreCall, DuringCall, PostCall])]
#[case::hooks_aporia(wire::APORIA, &[DuringCall, PostCall])]
#[case::hooks_azure_prompt_shield(wire::AZURE_PROMPT_SHIELD, &[PreCall, DuringCall])]
#[case::hooks_azure_text_moderations(wire::AZURE_TEXT_MODERATIONS, &[PreCall, PostCall])]
#[case::hooks_block_code_execution(wire::BLOCK_CODE_EXECUTION, &[PreCall, PostCall, DuringCall])]
#[case::hooks_cato_networks(wire::CATO_NETWORKS, &[PreCall, DuringCall, PostCall])]
#[case::hooks_cisco_ai_defense(
    wire::CISCO_AI_DEFENSE,
    &[PreCall, DuringCall, PostCall, LoggingOnly, PreMcpCall, DuringMcpCall]
)]
#[case::hooks_compresr(wire::COMPRESR, &[])]
#[case::hooks_conduct(wire::CONDUCT, &[])]
#[case::hooks_crowdstrike_aidr(wire::CROWDSTRIKE_AIDR, &[PreCall, PostCall])]
#[case::hooks_custom_code(
    wire::CUSTOM_CODE,
    &[PreCall, DuringCall, PostCall, PreMcpCall, DuringMcpCall, LoggingOnly]
)]
#[case::hooks_deep_keep(wire::DEEP_KEEP, &[PreCall, PostCall, DuringCall])]
#[case::hooks_dynamoai(wire::DYNAMOAI, &[PreCall, PostCall, DuringCall])]
#[case::hooks_enkryptai(wire::ENKRYPTAI, &[PreCall, PostCall, DuringCall])]
#[case::hooks_generic_guardrail_api(wire::GENERIC_GUARDRAIL_API, &[PreCall, PostCall, DuringCall])]
#[case::hooks_grayswan(wire::GRAYSWAN, &[PreCall, DuringCall, PostCall])]
#[case::hooks_guardrails_ai(wire::GUARDRAILS_AI, &[PostCall, PreCall, LoggingOnly])]
#[case::hooks_headroom(wire::HEADROOM, &[PreCall, PostCall])]
#[case::hooks_hiddenlayer(wire::HIDDENLAYER, &[PreCall, PostCall])]
#[case::hooks_ibm_guardrails(wire::IBM_GUARDRAILS, &[PreCall, PostCall, DuringCall])]
#[case::hooks_javelin(wire::JAVELIN, &[PreCall])]
#[case::hooks_lasso(wire::LASSO, &[PreCall, DuringCall, PostCall])]
#[case::hooks_litellm_content_filter(
    wire::LITELLM_CONTENT_FILTER,
    &[PreCall, PostCall, DuringCall, RealtimeInputTranscription, PreMcpCall, PostMcpCall]
)]
#[case::hooks_llm_as_a_judge(wire::LLM_AS_A_JUDGE, &[PreCall, DuringCall, PostCall])]
#[case::hooks_mcp_jwt_signer(wire::MCP_JWT_SIGNER, &[PreMcpCall])]
#[case::hooks_microsoft_purview(wire::MICROSOFT_PURVIEW, &[PreCall, PostCall, LoggingOnly])]
#[case::hooks_model_armor(
    wire::MODEL_ARMOR,
    &[PreCall, DuringCall, PostCall, PreMcpCall, DuringMcpCall, LoggingOnly]
)]
#[case::hooks_noma(wire::NOMA, &[PreCall, DuringCall, PostCall, PreMcpCall])]
#[case::hooks_noma_v2(wire::NOMA_V2, &[PreCall, DuringCall, PostCall, PreMcpCall, DuringMcpCall])]
#[case::hooks_onyx(wire::ONYX, &[PreCall, DuringCall, PostCall])]
#[case::hooks_openai_moderation(wire::OPENAI_MODERATION, &[PreCall, DuringCall, PostCall])]
#[case::hooks_ovalix(wire::OVALIX, &[PreCall, PostCall])]
#[case::hooks_pangea(wire::PANGEA, &[PreCall, PostCall])]
#[case::hooks_panw_prisma_airs(
    wire::PANW_PRISMA_AIRS,
    &[PreCall, DuringCall, PostCall, LoggingOnly, PreMcpCall, DuringMcpCall, PostMcpCall]
)]
#[case::hooks_pillar(wire::PILLAR, &[PreCall, DuringCall, PostCall, PreMcpCall, DuringMcpCall])]
#[case::hooks_promptguard(wire::PROMPTGUARD, &[PreCall, PostCall])]
#[case::hooks_qostodian_nexus(wire::QOSTODIAN_NEXUS, &[PreCall, PostCall, DuringCall])]
#[case::hooks_qualifire(wire::QUALIFIRE, &[PreCall, DuringCall, PostCall])]
#[case::hooks_repelloai(wire::REPELLOAI, &[PreCall, PostCall])]
#[case::hooks_rubrik(wire::RUBRIK, &[PreCall, PostCall])]
#[case::hooks_semantic_guard(wire::SEMANTIC_GUARD, &[PreCall, PostCall])]
#[case::hooks_singulr(wire::SINGULR, &[PreCall, PostCall, LoggingOnly, PreMcpCall, PostMcpCall])]
#[case::hooks_straiker(wire::STRAIKER, &[PreCall, PostCall])]
#[case::hooks_typesafe(wire::TYPESAFE, &[])]
#[case::hooks_vigil_guard(wire::VIGIL_GUARD, &[PreCall, PostCall])]
#[case::hooks_xecguard(wire::XECGUARD, &[PreCall, DuringCall, PostCall, LoggingOnly])]
#[case::hooks_zscaler_ai_guard(wire::ZSCALER_AI_GUARD, &[PreCall, PostCall])]
fn test_every_wire_name_builds_with_its_supported_hooks(
    #[case] kind: &str,
    #[case] hooks: &[GuardrailEventHooks],
) {
    let spec = build_ok(kind);
    let settings = spec.settings();
    assert_eq!(
        settings.guardrail_name.as_deref(),
        Some("test-guard"),
        "{kind}"
    );
    assert_eq!(
        settings.event_hook,
        Some(LitellmParamsMode::One(mode_of(kind).to_owned())),
        "{kind}"
    );
    assert!(settings.default_on, "{kind}");
    assert_eq!(settings.timeout, Some(7.5_f64), "{kind}");
    assert_eq!(supported_event_hooks(kind), hooks, "{kind}");
}

#[rstest]
#[case::a_aim(wire::AIM)]
#[case::a_akto(wire::AKTO)]
#[case::a_aporia(wire::APORIA)]
#[case::a_cisco_ai_defense(wire::CISCO_AI_DEFENSE)]
#[case::a_conduct(wire::CONDUCT)]
#[case::a_crowdstrike_aidr(wire::CROWDSTRIKE_AIDR)]
#[case::a_deep_keep(wire::DEEP_KEEP)]
#[case::a_generic_guardrail_api(wire::GENERIC_GUARDRAIL_API)]
#[case::a_grayswan(wire::GRAYSWAN)]
#[case::a_guardrails_ai(wire::GUARDRAILS_AI)]
#[case::a_headroom(wire::HEADROOM)]
#[case::a_lasso(wire::LASSO)]
fn test_third_party_a_arms_hold_their_own_args(#[case] kind: &str) {
    match kind {
        wire::AIM => assert!(matches!(&build_ok(kind).kind, HookKind::Aim(_)), "{kind}"),
        wire::AKTO => assert!(matches!(&build_ok(kind).kind, HookKind::Akto(_)), "{kind}"),
        wire::APORIA => assert!(
            matches!(&build_ok(kind).kind, HookKind::Aporia(_)),
            "{kind}"
        ),
        wire::CISCO_AI_DEFENSE => {
            assert!(
                matches!(&build_ok(kind).kind, HookKind::CiscoAiDefense(_)),
                "{kind}"
            );
        }
        wire::CONDUCT => assert!(
            matches!(&build_ok(kind).kind, HookKind::Conduct(_)),
            "{kind}"
        ),
        wire::CROWDSTRIKE_AIDR => {
            assert!(
                matches!(&build_ok(kind).kind, HookKind::CrowdstrikeAidr(_)),
                "{kind}"
            );
        }
        wire::DEEP_KEEP => {
            assert!(
                matches!(&build_ok(kind).kind, HookKind::Deepkeep(_)),
                "{kind}"
            );
        }
        wire::GENERIC_GUARDRAIL_API => {
            assert!(
                matches!(&build_ok(kind).kind, HookKind::GenericGuardrailApi(_)),
                "{kind}"
            );
        }
        wire::GRAYSWAN => assert!(
            matches!(&build_ok(kind).kind, HookKind::Grayswan(_)),
            "{kind}"
        ),
        wire::GUARDRAILS_AI => {
            assert!(
                matches!(&build_ok(kind).kind, HookKind::GuardrailsAi(_)),
                "{kind}"
            );
        }
        wire::HEADROOM => assert!(
            matches!(&build_ok(kind).kind, HookKind::Headroom(_)),
            "{kind}"
        ),
        wire::LASSO => assert!(matches!(&build_ok(kind).kind, HookKind::Lasso(_)), "{kind}"),
        _other => panic!("{kind} is not a third_party_a name"),
    }
}

#[rstest]
#[case::b_panw_prisma_airs(wire::PANW_PRISMA_AIRS)]
#[case::b_pillar(wire::PILLAR)]
#[case::b_rubrik(wire::RUBRIK)]
#[case::b_straiker(wire::STRAIKER)]
#[case::b_typesafe(wire::TYPESAFE)]
#[case::b_vigil_guard(wire::VIGIL_GUARD)]
#[case::b_xecguard(wire::XECGUARD)]
fn test_third_party_b_arms_hold_their_own_args(#[case] kind: &str) {
    match kind {
        wire::PANW_PRISMA_AIRS => {
            assert!(
                matches!(&build_ok(kind).kind, HookKind::PanwPrismaAirs(_)),
                "{kind}"
            );
        }
        wire::PILLAR => assert!(
            matches!(&build_ok(kind).kind, HookKind::Pillar(_)),
            "{kind}"
        ),
        wire::RUBRIK => assert!(
            matches!(&build_ok(kind).kind, HookKind::Rubrik(_)),
            "{kind}"
        ),
        wire::STRAIKER => assert!(
            matches!(&build_ok(kind).kind, HookKind::Straiker(_)),
            "{kind}"
        ),
        wire::TYPESAFE => assert!(
            matches!(&build_ok(kind).kind, HookKind::Typesafe(_)),
            "{kind}"
        ),
        wire::VIGIL_GUARD => {
            assert!(
                matches!(&build_ok(kind).kind, HookKind::VigilGuard(_)),
                "{kind}"
            );
        }
        wire::XECGUARD => assert!(
            matches!(&build_ok(kind).kind, HookKind::Xecguard(_)),
            "{kind}"
        ),
        _other => panic!("{kind} is not a third_party_b name"),
    }
}

#[rstest]
#[case::init_agent_365(wire::AGENT_365)]
#[case::init_azure_prompt_shield(wire::AZURE_PROMPT_SHIELD)]
#[case::init_azure_text_moderations(wire::AZURE_TEXT_MODERATIONS)]
#[case::init_ibm_guardrails(wire::IBM_GUARDRAILS)]
#[case::init_microsoft_purview(wire::MICROSOFT_PURVIEW)]
#[case::init_model_armor(wire::MODEL_ARMOR)]
#[case::init_openai_moderation(wire::OPENAI_MODERATION)]
fn test_hook_initializer_arms_hold_their_own_args(#[case] kind: &str) {
    match kind {
        wire::AGENT_365 => assert!(
            matches!(&build_ok(kind).kind, HookKind::Agent365(_)),
            "{kind}"
        ),
        wire::AZURE_PROMPT_SHIELD => {
            assert!(
                matches!(&build_ok(kind).kind, HookKind::AzurePromptShield(_)),
                "{kind}"
            );
        }
        wire::AZURE_TEXT_MODERATIONS => {
            assert!(
                matches!(&build_ok(kind).kind, HookKind::AzureTextModerations(_)),
                "{kind}"
            );
        }
        wire::IBM_GUARDRAILS => {
            assert!(
                matches!(&build_ok(kind).kind, HookKind::IbmGuardrails(_)),
                "{kind}"
            );
        }
        wire::MICROSOFT_PURVIEW => {
            assert!(
                matches!(&build_ok(kind).kind, HookKind::MicrosoftPurview(_)),
                "{kind}"
            );
        }
        wire::MODEL_ARMOR => assert!(
            matches!(&build_ok(kind).kind, HookKind::ModelArmor(_)),
            "{kind}"
        ),
        wire::OPENAI_MODERATION => {
            assert!(
                matches!(&build_ok(kind).kind, HookKind::OpenaiModeration(_)),
                "{kind}"
            );
        }
        _other => panic!("{kind} is not a hook initializer name"),
    }
}

#[rstest]
#[case::special_litellm_content_filter(wire::LITELLM_CONTENT_FILTER)]
#[case::special_mcp_jwt_signer(wire::MCP_JWT_SIGNER)]
fn test_content_filter_and_signer_arms_hold_their_own_args(#[case] kind: &str) {
    match kind {
        wire::LITELLM_CONTENT_FILTER => {
            assert!(
                matches!(&build_ok(kind).kind, HookKind::LitellmContentFilter(_)),
                "{kind}"
            );
        }
        wire::MCP_JWT_SIGNER => {
            assert!(
                matches!(&build_ok(kind).kind, HookKind::McpJwtSigner(_)),
                "{kind}"
            );
        }
        _other => panic!("{kind} is not a content filter or signer name"),
    }
}

#[rstest]
#[case::vendor_promptguard(wire::PROMPTGUARD)]
#[case::vendor_qostodian_nexus(wire::QOSTODIAN_NEXUS)]
#[case::vendor_qualifire(wire::QUALIFIRE)]
#[case::vendor_semantic_guard(wire::SEMANTIC_GUARD)]
#[case::vendor_singulr(wire::SINGULR)]
#[case::vendor_zscaler_ai_guard(wire::ZSCALER_AI_GUARD)]
fn test_vendor_initializer_arms_hold_their_own_args(#[case] kind: &str) {
    match kind {
        wire::PROMPTGUARD => {
            assert!(
                matches!(&build_ok(kind).kind, HookKind::Promptguard(_)),
                "{kind}"
            );
        }
        wire::QOSTODIAN_NEXUS => {
            assert!(
                matches!(&build_ok(kind).kind, HookKind::QostodianNexus(_)),
                "{kind}"
            );
        }
        wire::QUALIFIRE => assert!(
            matches!(&build_ok(kind).kind, HookKind::Qualifire(_)),
            "{kind}"
        ),
        wire::SEMANTIC_GUARD => {
            assert!(
                matches!(&build_ok(kind).kind, HookKind::SemanticGuard(_)),
                "{kind}"
            );
        }
        wire::SINGULR => assert!(
            matches!(&build_ok(kind).kind, HookKind::Singulr(_)),
            "{kind}"
        ),
        wire::ZSCALER_AI_GUARD => {
            assert!(
                matches!(&build_ok(kind).kind, HookKind::ZscalerAiGuard(_)),
                "{kind}"
            );
        }
        _other => panic!("{kind} is not a vendor initializer name"),
    }
}

#[rstest]
#[case::class_block_code_execution(wire::BLOCK_CODE_EXECUTION)]
#[case::class_compresr(wire::COMPRESR)]
#[case::class_custom_code(wire::CUSTOM_CODE)]
#[case::class_hiddenlayer(wire::HIDDENLAYER)]
#[case::class_javelin(wire::JAVELIN)]
#[case::class_llm_as_a_judge(wire::LLM_AS_A_JUDGE)]
#[case::class_noma(wire::NOMA)]
#[case::class_noma_v2(wire::NOMA_V2)]
#[case::class_onyx(wire::ONYX)]
#[case::class_ovalix(wire::OVALIX)]
#[case::class_pangea(wire::PANGEA)]
#[case::class_repelloai(wire::REPELLOAI)]
fn test_class_arms_keep_their_params(#[case] kind: &str) {
    match kind {
        wire::BLOCK_CODE_EXECUTION => {
            assert!(
                matches!(&build_ok(kind).kind, HookKind::BlockCodeExecution(_)),
                "{kind}"
            );
        }
        wire::COMPRESR => assert!(
            matches!(&build_ok(kind).kind, HookKind::Compresr(_)),
            "{kind}"
        ),
        wire::CUSTOM_CODE => {
            assert!(
                matches!(&build_ok(kind).kind, HookKind::CustomCode(_)),
                "{kind}"
            );
        }
        wire::HIDDENLAYER => {
            assert!(
                matches!(&build_ok(kind).kind, HookKind::Hiddenlayer(_)),
                "{kind}"
            );
        }
        wire::JAVELIN => assert!(
            matches!(&build_ok(kind).kind, HookKind::Javelin(_)),
            "{kind}"
        ),
        wire::LLM_AS_A_JUDGE => {
            assert!(
                matches!(&build_ok(kind).kind, HookKind::LlmAsAJudge(_)),
                "{kind}"
            );
        }
        wire::NOMA => assert!(matches!(&build_ok(kind).kind, HookKind::Noma(_)), "{kind}"),
        wire::NOMA_V2 => assert!(
            matches!(&build_ok(kind).kind, HookKind::NomaV2(_)),
            "{kind}"
        ),
        wire::ONYX => assert!(matches!(&build_ok(kind).kind, HookKind::Onyx(_)), "{kind}"),
        wire::OVALIX => assert!(
            matches!(&build_ok(kind).kind, HookKind::Ovalix(_)),
            "{kind}"
        ),
        wire::PANGEA => assert!(
            matches!(&build_ok(kind).kind, HookKind::Pangea(_)),
            "{kind}"
        ),
        wire::REPELLOAI => {
            assert!(
                matches!(&build_ok(kind).kind, HookKind::Repelloai(_)),
                "{kind}"
            );
        }
        _other => panic!("{kind} is not a class name"),
    }
}

#[rstest]
#[case::ported_alice(wire::ALICE)]
#[case::ported_cato_networks(wire::CATO_NETWORKS)]
#[case::ported_dynamoai(wire::DYNAMOAI)]
#[case::ported_enkryptai(wire::ENKRYPTAI)]
fn test_ported_arms_hold_their_own_args(#[case] kind: &str) {
    match kind {
        wire::ALICE => assert!(matches!(&build_ok(kind).kind, HookKind::Alice(_)), "{kind}"),
        wire::CATO_NETWORKS => {
            assert!(
                matches!(&build_ok(kind).kind, HookKind::CatoNetworks(_)),
                "{kind}"
            );
        }
        wire::DYNAMOAI => assert!(
            matches!(&build_ok(kind).kind, HookKind::Dynamoai(_)),
            "{kind}"
        ),
        wire::ENKRYPTAI => {
            assert!(
                matches!(&build_ok(kind).kind, HookKind::Enkryptai(_)),
                "{kind}"
            );
        }
        _other => panic!("{kind} is not a ported name"),
    }
}

#[rstest]
#[case::unknown_custom_module("my_module.MyGuardrail")]
#[case::unknown_bedrock("bedrock")]
#[case::unknown_tool_policy("tool_policy")]
#[case::unknown_empty("")]
fn test_unknown_wire_name_is_not_built(#[case] kind: &str) {
    let params = test_params(wire::AIM);
    assert!(
        build_hook_spec(kind, &params, "test-guard", &test_ports()).is_none(),
        "{kind}"
    );
}

#[test]
fn factory_without_hooks_still_answers_unsupported() {
    let params = test_params(wire::AIM);
    let error = factories()
        .build(wire::AIM, &params, "test-guard", true)
        .err()
        .expect("no builder");
    assert_eq!(error.to_string(), "Unsupported guardrail: aim");
    let RegistryError::Unsupported { kind } = error else {
        panic!("unsupported guardrail expected");
    };
    assert_eq!(kind, wire::AIM);
}

#[test]
fn factory_with_hooks_rejects_unknown_names() {
    let params = params_from(r#"{"guardrail":"my_module.MyGuardrail","mode":"pre_call"}"#);
    let error = factories()
        .with_hooks(test_ports())
        .build("my_module.MyGuardrail", &params, "custom", true)
        .err()
        .expect("unknown kind");
    assert_eq!(
        error.to_string(),
        "Unsupported guardrail: my_module.MyGuardrail"
    );
}

#[test]
fn factory_with_hooks_builds_a_hook_callback() {
    let params = test_params(wire::AIM);
    let mut callbacks = factories()
        .with_hooks(test_ports())
        .build(wire::AIM, &params, "aim-guard", true)
        .unwrap();
    let callback = callbacks.remove(0);
    let CallbackSpec::Hook(spec) = &callback.spec else {
        panic!("hook callback expected");
    };
    assert!(matches!(&spec.kind, HookKind::Aim(_)));
    let settings = &callback.base.settings;
    assert_eq!(settings.guardrail_name.as_deref(), Some("aim-guard"));
    assert_eq!(
        settings.supported_event_hooks,
        Some(vec![PreCall, DuringCall, PostCall])
    );
    assert_eq!(
        settings.event_hook,
        Some(LitellmParamsMode::One("pre_call".to_owned()))
    );
    assert!(settings.default_on);
    assert_eq!(settings.timeout, Some(7.5_f64));
    assert!(callback.supports_scan_only_tool_results());
    assert!(!callback.is_compression());
}

#[test]
fn prisma_airs_refuses_scan_only_tool_results() {
    let builder = factories().with_hooks(test_ports());
    let panw = test_params(wire::PANW_PRISMA_AIRS);
    let panw_callbacks = builder
        .build(wire::PANW_PRISMA_AIRS, &panw, "panw-guard", true)
        .unwrap();
    assert!(!panw_callbacks[0].supports_scan_only_tool_results());
    let aim = test_params(wire::AIM);
    let aim_callbacks = builder.build(wire::AIM, &aim, "aim-guard", true).unwrap();
    assert!(aim_callbacks[0].supports_scan_only_tool_results());
}

#[test]
fn initializer_refusal_keeps_its_text() {
    let params = params_from(r#"{"guardrail":"guardrails_ai","mode":"pre_call"}"#);
    let error = build_error_on(wire::GUARDRAILS_AI, &params);
    assert_eq!(
        error.to_string(),
        "GuardrailsAIException - Please pass the Guardrails AI guard name via 'litellm_params::guard_name'"
    );
    let HookRegistryError::Init { kind, .. } = error else {
        panic!("initializer error expected");
    };
    assert_eq!(kind, wire::GUARDRAILS_AI);
}

#[test]
fn unbuilt_decision_model() {
    let error = build_error(wire::DECISION_MODEL);
    assert_eq!(
        error.to_string(),
        "support for the decision_model guardrail is not built yet"
    );
    let HookRegistryError::NotBuilt { kind } = error else {
        panic!("not-built error expected");
    };
    assert_eq!(kind, wire::DECISION_MODEL);
}

#[test]
fn unbuilt_prompt_security() {
    let error = build_error(wire::PROMPT_SECURITY);
    assert_eq!(
        error.to_string(),
        "support for the prompt_security guardrail is not built yet"
    );
    let HookRegistryError::NotBuilt { kind } = error else {
        panic!("not-built error expected");
    };
    assert_eq!(kind, wire::PROMPT_SECURITY);
}

#[test]
fn unbuilt_mcp_end_user_permission() {
    let error = build_error(wire::MCP_END_USER_PERMISSION);
    assert_eq!(
        error.to_string(),
        "support for the mcp_end_user_permission guardrail is not built yet"
    );
    let HookRegistryError::NotBuilt { kind } = error else {
        panic!("not-built error expected");
    };
    assert_eq!(kind, wire::MCP_END_USER_PERMISSION);
}

#[test]
fn unbuilt_mcp_security() {
    let error = build_error(wire::MCP_SECURITY);
    assert_eq!(
        error.to_string(),
        "support for the mcp_security guardrail is not built yet"
    );
    let HookRegistryError::NotBuilt { kind } = error else {
        panic!("not-built error expected");
    };
    assert_eq!(kind, wire::MCP_SECURITY);
}

#[test]
fn hook_errors_at_startup() {
    let init = RegistryError::Hook(HookRegistryError::Init {
        kind: wire::AIM.to_owned(),
        message: "boom".to_owned(),
    });
    let unbuilt = RegistryError::Hook(HookRegistryError::NotBuilt {
        kind: wire::DECISION_MODEL.to_owned(),
    });
    assert!(skips_startup(&init));
    assert!(!skips_startup(&unbuilt));
}

#[test]
fn azure_names_share_one_initializer() {
    let spec = build_ok(wire::AZURE_PROMPT_SHIELD);
    let HookKind::AzurePromptShield(shield) = &spec.kind else {
        panic!("prompt shield spec");
    };
    let AzureArgs::PromptShield(common) = shield.as_ref() else {
        panic!("prompt shield args");
    };
    assert_eq!(common.guardrail_name, "test-guard");

    let moderations_spec = build_ok(wire::AZURE_TEXT_MODERATIONS);
    let HookKind::AzureTextModerations(moderations) = &moderations_spec.kind else {
        panic!("text moderations spec");
    };
    let AzureArgs::TextModerations(text_common) = moderations.as_ref() else {
        panic!("text moderations args");
    };
    assert_eq!(text_common.guardrail_name, "test-guard");
}

#[test]
fn agent_365_args_keep_the_tenant_identity() {
    let spec = build_ok(wire::AGENT_365);
    let HookKind::Agent365(args) = &spec.kind else {
        panic!("agent 365 spec");
    };
    assert_eq!(args.guardrail_name, "test-guard");
    assert_eq!(args.tenant_id, "tenant");
    assert_eq!(args.client_id, "client");
    assert_eq!(exposed(Some(&args.client_secret)), Some("secret"));
}

#[test]
fn agent_365_reports_the_overrides_it_ignores() {
    let sink = LogSink::default();
    let writer = sink.clone();
    let subscriber = fmt()
        .with_max_level(tracing::Level::WARN)
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    let guard = set_default(subscriber);
    let spec = build_ok(wire::AGENT_365);
    drop(guard);
    assert!(matches!(&spec.kind, HookKind::Agent365(_)));
    let printed = String::from_utf8(sink.0.lock().expect("the captured log").clone())
        .expect("the captured log is UTF-8");
    assert!(printed.contains("ignoring api_base"), "{printed}");
}

#[test]
fn agent_365_stays_quiet_when_nothing_is_ignored() {
    let sink = LogSink::default();
    let writer = sink.clone();
    let subscriber = fmt()
        .with_max_level(tracing::Level::WARN)
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    let guard = set_default(subscriber);
    let params = params_from(
        r#"{"guardrail":"agent_365","mode":"pre_mcp_call","tenant_id":"tenant","client_id":"client","client_secret":"secret"}"#,
    );
    let spec = build_hook_spec(wire::AGENT_365, &params, "quiet-guard", &test_ports())
        .expect("wire name is built")
        .expect("agent 365 builds");
    drop(guard);
    assert!(matches!(&spec.kind, HookKind::Agent365(_)));
    let printed = String::from_utf8(sink.0.lock().expect("the captured log").clone())
        .expect("the captured log is UTF-8");
    assert!(!printed.contains("ignoring"), "{printed}");
}

#[test]
fn mcp_jwt_signer_registers_the_signer_it_builds() {
    let ports = test_ports();
    let params = test_params(wire::MCP_JWT_SIGNER);
    let spec = build_hook_spec(wire::MCP_JWT_SIGNER, &params, "signer-guard", &ports)
        .expect("wire name is built")
        .expect("signer builds");
    let HookKind::McpJwtSigner(signer) = &spec.kind else {
        panic!("signer spec expected");
    };
    let slot = ports.slot.get().expect("slot holds the signer");
    assert!(Arc::ptr_eq(&slot, signer));
    assert_eq!(ports.registered.lock().unwrap().len(), 1);
}

#[test]
fn mcp_jwt_signer_keeps_the_hook_port_client() {
    let mut headers = HeaderMap::new();
    headers.insert(
        "x-betterlite-port",
        HeaderValue::from_static("hook-registry"),
    );
    let client = base_builder()
        .default_headers(headers)
        .build()
        .expect("a port client");
    let mut ports = test_ports();
    ports.client = client;
    let params = test_params(wire::MCP_JWT_SIGNER);
    let spec = build_hook_spec(wire::MCP_JWT_SIGNER, &params, "signer-guard", &ports)
        .expect("wire name is built")
        .expect("signer builds");
    let HookKind::McpJwtSigner(signer) = &spec.kind else {
        panic!("signer spec expected");
    };
    assert!(
        format!("{:?}", signer.idp.client).contains("x-betterlite-port"),
        "the signer keeps the client the hook port serves"
    );
}

#[test]
fn mcp_jwt_signer_refuses_another_mode() {
    let params = params_from(r#"{"guardrail":"mcp_jwt_signer","mode":"pre_call"}"#);
    let error = build_error_on(wire::MCP_JWT_SIGNER, &params);
    assert!(
        error.to_string().contains("must use mode='pre_mcp_call'"),
        "{error}"
    );
    let HookRegistryError::Init { kind, .. } = error else {
        panic!("initializer error expected");
    };
    assert_eq!(kind, wire::MCP_JWT_SIGNER);
}

#[test]
fn content_filter_requires_a_guardrail_name() {
    let params = test_params(wire::LITELLM_CONTENT_FILTER);
    let error = build_hook_spec(wire::LITELLM_CONTENT_FILTER, &params, "", &test_ports())
        .expect("wire name is built")
        .err()
        .expect("empty name fails");
    assert_eq!(
        error.to_string(),
        "Content Filter: guardrail_name is required"
    );
    assert!(matches!(error, HookRegistryError::Init { .. }));
}

#[rstest]
#[case::router_present(true)]
#[case::router_absent(false)]
fn test_content_filter_passes_the_router_port(#[case] present: bool) {
    let mut ports = test_ports();
    ports.router_present = present;
    let params = test_params(wire::LITELLM_CONTENT_FILTER);
    let spec = build_hook_spec(wire::LITELLM_CONTENT_FILTER, &params, "test-guard", &ports)
        .expect("wire name is built")
        .expect("content filter builds");
    let HookKind::LitellmContentFilter(args) = &spec.kind else {
        panic!("content filter spec expected");
    };
    assert_eq!(args.router_present, present);
}

#[test]
fn semantic_guard_needs_the_router() {
    let mut ports = test_ports();
    ports.router_present = false;
    let params = test_params(wire::SEMANTIC_GUARD);
    let error = build_hook_spec(wire::SEMANTIC_GUARD, &params, "test-guard", &ports)
        .expect("wire name is built")
        .err()
        .expect("missing router fails");
    assert_eq!(
        error.to_string(),
        "SemanticGuard requires llm_router for embeddings. Configure a model_list with an embedding model."
    );
}

#[derive(Clone, Default)]
struct LogSink(Arc<Mutex<Vec<u8>>>);

impl Write for LogSink {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|_poisoned| io::Error::other("the log lock is poisoned"))?
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

mod ported;
