use std::collections::BTreeMap;

use betterlite_config::loader::ConfigLoader;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use rstest::rstest;
use secrecy::ExposeSecret;

#[rstest]
#[case::publicai(ProviderId::Publicai, "PUBLICAI_API_KEY")]
#[case::helicone(ProviderId::Helicone, "HELICONE_API_KEY")]
#[case::veniceai(ProviderId::Veniceai, "VENICE_AI_API_KEY")]
#[case::xiaomi_mimo(ProviderId::XiaomiMimo, "XIAOMI_MIMO_API_KEY")]
#[case::scaleway(ProviderId::Scaleway, "SCW_SECRET_KEY")]
#[case::synthetic(ProviderId::Synthetic, "SYNTHETIC_API_KEY")]
#[case::apertis(ProviderId::Apertis, "STIMA_API_KEY")]
#[case::nano_gpt(ProviderId::Nanogpt, "NANOGPT_API_KEY")]
#[case::poe(ProviderId::Poe, "POE_API_KEY")]
#[case::chutes(ProviderId::Chutes, "CHUTES_API_KEY")]
#[case::abliteration(ProviderId::Abliteration, "ABLITERATION_API_KEY")]
#[case::llamagate(ProviderId::Llamagate, "LLAMAGATE_API_KEY")]
#[case::gmi(ProviderId::Gmi, "GMI_API_KEY")]
#[case::sarvam(ProviderId::Sarvam, "SARVAM_API_KEY")]
#[case::assemblyai(ProviderId::Assemblyai, "ASSEMBLYAI_API_KEY")]
#[case::charity_engine(ProviderId::CharityEngine, "CHARITY_ENGINE_API_KEY")]
#[case::aihubmix(ProviderId::Aihubmix, "AIHUBMIX_API_KEY")]
#[case::crusoe(ProviderId::Crusoe, "CRUSOE_API_KEY")]
#[case::darkbloom(ProviderId::Darkbloom, "DARKBLOOM_API_KEY")]
#[case::neosantara(ProviderId::Neosantara, "NEOSANTARA_API_KEY")]
#[case::tensormesh(ProviderId::Tensormesh, "TENSORMESH_INFERENCE_API_KEY")]
#[case::parasail(ProviderId::Parasail, "PARASAIL_API_KEY")]
#[case::libertai(ProviderId::Libertai, "LIBERTAI_API_KEY")]
#[case::empiriolabs(ProviderId::Empiriolabs, "EMPIRIOLABS_API_KEY")]
#[case::meta(ProviderId::Meta, "META_API_KEY")]
#[case::cognition(ProviderId::Cognition, "COGNITION_API_KEY")]
#[case::cortecs(ProviderId::Cortecs, "CORTECS_API_KEY")]
#[case::pinstripes(ProviderId::Pinstripes, "PINSTRIPES_API_KEY")]
#[case::scx_ai(ProviderId::ScxAi, "SCX_API_KEY")]
#[case::prism(ProviderId::Prism, "PRISM_API_KEY")]
#[case::sail(ProviderId::Sail, "SAIL_API_KEY")]
fn json_provider_settings_resolve_environment_key(
    #[case] provider: ProviderId,
    #[case] expected_key: &str,
) {
    let registry = JSONProviderRegistry::load();
    let descriptor = registry.providers.get(&provider).expect("pinned provider");
    assert_eq!(descriptor.api_key_env, expected_key);
    let mut config =
        format!("environment_variables:\n  {expected_key}: os.environ/{expected_key}\n");
    let mut environment =
        BTreeMap::from([(expected_key.to_owned(), format!("{expected_key}-value"))]);
    if let Some(base_key) = descriptor.api_base_env.as_deref() {
        config.push_str(&format!("  {base_key}: os.environ/{base_key}\n"));
        environment.insert(base_key.to_owned(), format!("{base_key}-override"));
    }
    let loaded = ConfigLoader {
        lookup: environment,
    }
    .load(&config)
    .expect("environment config");
    assert_eq!(
        loaded
            .environment_variables
            .get(expected_key)
            .and_then(Option::as_ref)
            .map(ExposeSecret::expose_secret),
        Some(format!("{expected_key}-value").as_str())
    );
    assert!(
        !loaded
            .environment_variables
            .contains_key("UNSUPPORTED_JSON_PROVIDER_KEY")
    );
}
