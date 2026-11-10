#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::gdc_settings::{BoolOrString, Settings};
use rstest::rstest;

const CONFIG: &str = "\
model_list:
  - model_name: gdc-chat
    litellm_params:
      model: gdc/gemini-2.5-pro
      vertex_project: call-project
      vertex_ai_project: call-alias-project
      vertex_location: call-location
      vertex_ai_location: call-alias-location
      ssl_verify: false
      gdc_token_caching: \"yes\"
  - model_name: gdc-plain
    litellm_params:
      model: gdc/gemini-2.5-flash
litellm_settings:
  api_base: https://api.example
  gdc_api_base: https://gdc.example
  vertex_project: global-project
  vertex_ai_project: global-alias-project
  vertex_location: global-location
  vertex_ai_location: global-alias-location
";

fn load(environment: &[(&str, &str)]) -> Settings {
    let lookup = environment
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    ConfigLoader { lookup }
        .load(CONFIG)
        .expect("the GDC config loads")
        .sections
        .gdc_settings
}

#[rstest]
#[case::both(&[("SSL_VERIFY", "false"), ("GDC_TOKEN_CACHING", "yes")], Some("false"), Some("yes"))]
#[case::verify_only(&[("SSL_VERIFY", "ca.pem")], Some("ca.pem"), None)]
#[case::caching_only(&[("GDC_TOKEN_CACHING", "1")], None, Some("1"))]
#[case::empty_values_count_as_unset(&[("SSL_VERIFY", ""), ("GDC_TOKEN_CACHING", "")], None, None)]
#[case::absent(&[], None, None)]
fn loads_both_environment_keys(
    #[case] environment: &[(&str, &str)],
    #[case] verify: Option<&str>,
    #[case] caching: Option<&str>,
) {
    let loaded = load(environment);
    assert_eq!(loaded.ssl_verify_environment.as_deref(), verify);
    assert_eq!(loaded.gdc_token_caching_environment.as_deref(), caching);
}

#[test]
fn loads_request_ssl_verify_and_token_caching() {
    let loaded = load(&[]);
    let call = &loaded.deployments[0];
    assert_eq!(call.ssl_verify, Some(BoolOrString::Bool(false)));
    assert_eq!(
        call.gdc_token_caching,
        Some(BoolOrString::Text("yes".to_owned()))
    );
    let untuned = &loaded.deployments[1];
    assert_eq!(untuned.ssl_verify, None);
    assert_eq!(untuned.gdc_token_caching, None);
}

#[test]
fn loads_global_base_project_and_location() {
    let global = load(&[]).global;
    assert_eq!(global.api_base.as_deref(), Some("https://api.example"));
    assert_eq!(global.gdc_api_base.as_deref(), Some("https://gdc.example"));
    assert_eq!(global.vertex_project.as_deref(), Some("global-project"));
    assert_eq!(global.vertex_location.as_deref(), Some("global-location"));
}

#[test]
fn loads_every_vertex_alias_per_call_and_globally() {
    let loaded = load(&[]);
    let call = &loaded.deployments[0];
    assert_eq!(
        call.vertex_ai_project.as_deref(),
        Some("call-alias-project")
    );
    assert_eq!(
        call.vertex_ai_location.as_deref(),
        Some("call-alias-location")
    );
    assert_eq!(
        loaded.global.vertex_ai_project.as_deref(),
        Some("global-alias-project")
    );
    assert_eq!(
        loaded.global.vertex_ai_location.as_deref(),
        Some("global-alias-location")
    );
}

#[test]
fn request_values_stay_distinct_from_litellm_settings() {
    let loaded = load(&[]);
    let call = &loaded.deployments[0];
    assert_eq!(call.vertex_project.as_deref(), Some("call-project"));
    assert_eq!(
        loaded.global.vertex_project.as_deref(),
        Some("global-project")
    );
    assert_ne!(call.vertex_project, loaded.global.vertex_project);
    assert_eq!(call.vertex_location.as_deref(), Some("call-location"));
    assert_eq!(
        loaded.global.vertex_location.as_deref(),
        Some("global-location")
    );
    assert_ne!(call.vertex_location, loaded.global.vertex_location);
}
