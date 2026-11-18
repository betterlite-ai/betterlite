#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::ConfigLoader;
use crate::sections::tier_artifact_predictor::artifact::{
    Artifact, CohortStatistic, Dataset, DomainStatistic, GlobalStatistic,
};
use crate::sections::tier_artifact_predictor::{ArtifactSource, Config, Settings};

fn settings(source: &str, variables: &[(&str, &str)]) -> Result<Settings, String> {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load(source)
        .map(|config| config.sections.tier_artifact_predictor)
        .map_err(|error| error.to_string())
}

fn deployment(params: &str) -> String {
    format!(
        "model_list:\n  - model_name: router\n    litellm_params:\n      model: openai/gpt-4o\n{params}"
    )
}

fn router(fields: &str) -> String {
    deployment(&format!("      complexity_router_config:\n{fields}"))
}

fn only(fields: &str, variables: &[(&str, &str)]) -> Result<Option<Config>, String> {
    let mut deployments = settings(&router(fields), variables)?.deployments;
    let count = deployments.len();
    if count != 1 {
        return Err(format!("one deployment expected, got {count}"));
    }
    Ok(deployments.pop().flatten())
}

const GLOBAL_STATISTICS: &str = "          global_statistics:
            - tier: 1
              successes: 8
              observations: 10
            - tier: 2
              successes: 6
              observations: 10
            - tier: 3
              successes: 4
              observations: 10
            - tier: 4
              successes: 2
              observations: 10
";

fn global_statistics() -> Vec<GlobalStatistic> {
    [(1, 8.0_f64), (2, 6.0), (3, 4.0), (4, 2.0)]
        .into_iter()
        .map(|(tier, successes)| GlobalStatistic {
            tier,
            successes,
            observations: 10.0,
        })
        .collect()
}

#[test]
fn a_config_without_a_model_list_has_no_deployments() {
    assert_eq!(settings("{}", &[]).unwrap(), Settings::default());
    assert_eq!(
        settings("model_list: []", &[]).unwrap(),
        Settings::default()
    );
}

#[test]
fn a_deployment_without_the_router_config_carries_none() {
    let source = "model_list:\n  - model_name: primary\n    litellm_params:\n      model: gpt-4o\n";
    assert_eq!(settings(source, &[]).unwrap().deployments, [None]);
}

#[test]
fn deployments_stay_aligned_with_the_model_list() {
    let source = "model_list:
  - model_name: first
    litellm_params:
      model: gpt-4o
  - model_name: second
    litellm_params:
      complexity_router_config:
        heuristic_v2_success_threshold: 0.5
  - model_name: third
    litellm_params:
      model: gpt-4o-mini
";
    let loaded = settings(source, &[]).unwrap();
    assert_eq!(loaded.deployments.len(), 3);
    assert_eq!(loaded.deployments[0], None);
    assert_eq!(
        loaded.deployments[1],
        Some(Config {
            artifact: ArtifactSource::default(),
            success_threshold: Some(0.5_f64),
        })
    );
    assert_eq!(loaded.deployments[2], None);
}

#[rstest]
#[case::absent("", ArtifactSource::default())]
#[case::null("        heuristic_v2_artifact: null\n", ArtifactSource::default())]
#[case::builtin(
    "        heuristic_v2_artifact: ultrafeedback\n",
    ArtifactSource::BuiltIn("ultrafeedback".to_owned())
)]
fn the_artifact_defaults_to_the_packaged_ultrafeedback(
    #[case] fields: &str,
    #[case] expected: ArtifactSource,
) {
    let configured = only(fields, &[]).unwrap().unwrap();
    assert_eq!(configured.artifact, expected);
    assert_eq!(configured.success_threshold, None);
}

#[test]
fn an_unknown_builtin_artifact_is_refused() {
    let error = only("        heuristic_v2_artifact: arc_agi\n", &[]).unwrap_err();
    assert!(error.contains("invalid scalar"), "got {error}");
}

fn inline_field_yaml() -> String {
    format!(
        "        heuristic_v2_artifact:
          schema_version: 1
{GLOBAL_STATISTICS}          domain_statistics:
            - tier: 1
              successes: 3
              observations: 4
              request_type: code_generation
          cohort_statistics:
            - tier: 1
              successes: 1
              observations: 2
              cohort: \"code_generation|short|code=1|math=0|mc=0|intl=0\"
          domain_prior_mass: 150
          cohort_prior_mass: 10
          routing_threshold: 0.6
          datasets:
            - name: ultrafeedback
              url: https://example.test/ultrafeedback
              license: MIT
              rows: 120
          success_definition: quality score at least 4
          split_method: sha256 split
        heuristic_v2_success_threshold: 0.55
"
    )
}

fn inline_artifact() -> Artifact {
    Artifact {
        schema_version: 1,
        global_statistics: global_statistics(),
        domain_statistics: vec![DomainStatistic {
            tier: 1,
            successes: 3.0,
            observations: 4.0,
            request_type: "code_generation".to_owned(),
        }],
        cohort_statistics: vec![CohortStatistic {
            tier: 1,
            successes: 1.0,
            observations: 2.0,
            cohort: "code_generation|short|code=1|math=0|mc=0|intl=0".to_owned(),
        }],
        domain_prior_mass: 150.0,
        cohort_prior_mass: 10.0,
        routing_threshold: 0.6,
        datasets: vec![Dataset {
            name: "ultrafeedback".to_owned(),
            url: "https://example.test/ultrafeedback".to_owned(),
            license: "MIT".to_owned(),
            rows: 120,
            success_definition: "quality score meets the dataset success threshold".to_owned(),
        }],
        success_definition: "quality score at least 4".to_owned(),
        split_method: "sha256 split".to_owned(),
    }
}

#[test]
fn an_inline_artifact_resolves_every_key() {
    let configured = only(&inline_field_yaml(), &[]).unwrap().unwrap();
    assert_eq!(
        configured,
        Some(Config {
            artifact: ArtifactSource::Inline(Box::new(inline_artifact())),
            success_threshold: Some(0.55_f64),
        })
    );
}

#[test]
fn an_inline_artifact_keeps_the_declared_defaults() {
    let fields = format!("        heuristic_v2_artifact:\n{GLOBAL_STATISTICS}");
    let configured = only(&fields, &[]).unwrap().unwrap();
    let ArtifactSource::Inline(artifact) = configured.artifact else {
        panic!("the inline artifact stays inline");
    };
    assert_eq!(
        *artifact,
        Artifact {
            schema_version: 1,
            global_statistics: global_statistics(),
            domain_statistics: Vec::new(),
            cohort_statistics: Vec::new(),
            domain_prior_mass: 200.0,
            cohort_prior_mass: 20.0,
            routing_threshold: 0.75,
            datasets: Vec::new(),
            success_definition: "quality score meets the dataset success threshold".to_owned(),
            split_method: "sha256(prompt): 70% train, 15% validation, 15% test".to_owned(),
        }
    );
}

#[test]
fn an_inline_artifact_needs_its_global_statistics() {
    let error = only(
        "        heuristic_v2_artifact:\n          routing_threshold: 0.5\n",
        &[],
    )
    .unwrap_err();
    assert!(
        error.contains("a built-in artifact name or a tier artifact object"),
        "got {error}"
    );
}

#[test]
fn an_undeclared_artifact_key_is_ignored() {
    let fields =
        format!("        heuristic_v2_artifact:\n{GLOBAL_STATISTICS}          undeclared: 3\n");
    let configured = only(&fields, &[]).unwrap().unwrap();
    let ArtifactSource::Inline(artifact) = configured.artifact else {
        panic!("the inline artifact stays inline");
    };
    assert_eq!(artifact.global_statistics, global_statistics());
}

#[rstest]
#[case::number("        heuristic_v2_artifact: 3\n")]
#[case::list("        heuristic_v2_artifact: [ultrafeedback]\n")]
fn a_value_that_is_neither_a_name_nor_an_artifact_is_refused(#[case] fields: &str) {
    let error = only(fields, &[]).unwrap_err();
    assert!(
        error.contains("a built-in artifact name or a tier artifact object"),
        "got {error}"
    );
}

#[test]
fn an_inline_artifact_text_field_resolves_from_the_environment() {
    let fields = format!(
        "        heuristic_v2_artifact:
{GLOBAL_STATISTICS}          success_definition: os.environ/SUCCESS_DEFINITION
"
    );
    let configured = only(&fields, &[("SUCCESS_DEFINITION", "score >= 4")])
        .unwrap()
        .unwrap();
    let ArtifactSource::Inline(artifact) = configured.artifact else {
        panic!("the inline artifact stays inline");
    };
    assert_eq!(artifact.success_definition, "score >= 4");
}

#[test]
fn an_inline_artifact_number_does_not_resolve_from_the_environment() {
    let body = "          global_statistics:
            - tier: 1
              successes: os.environ/TIER_ONE_SUCCESSES
              observations: 10
            - tier: 2
              successes: 6
              observations: 10
            - tier: 3
              successes: 4
              observations: 10
            - tier: 4
              successes: 2
              observations: 10
";
    let fields = format!("        heuristic_v2_artifact:\n{body}");
    let error = only(&fields, &[("TIER_ONE_SUCCESSES", "8")]).unwrap_err();
    assert!(!error.is_empty());
}

#[rstest]
#[case::absent("", None)]
#[case::null("        heuristic_v2_success_threshold: null\n", None)]
#[case::zero("        heuristic_v2_success_threshold: 0\n", Some(0.0_f64))]
#[case::one("        heuristic_v2_success_threshold: 1\n", Some(1.0_f64))]
#[case::fraction("        heuristic_v2_success_threshold: 0.875\n", Some(0.875_f64))]
fn the_threshold_bounds_are_inclusive(#[case] fields: &str, #[case] expected: Option<f64>) {
    let configured = only(fields, &[]).unwrap().unwrap();
    assert_eq!(configured.success_threshold, expected);
}

#[rstest]
#[case::below("        heuristic_v2_success_threshold: -0.1\n")]
#[case::above("        heuristic_v2_success_threshold: 1.0000000000000002\n")]
#[case::continuous("        heuristic_v2_success_threshold: 1.1\n")]
fn a_threshold_outside_the_unit_interval_is_refused(#[case] fields: &str) {
    let error = only(fields, &[]).unwrap_err();
    assert!(error.contains("invalid scalar"), "got {error}");
}

#[test]
fn a_quoted_threshold_is_rejected() {
    let error = only("        heuristic_v2_success_threshold: \"0.5\"\n", &[]).unwrap_err();
    assert!(error.contains("invalid scalar"), "got {error}");
}

#[test]
fn the_threshold_path_names_the_deployment_and_key() {
    let error = only("        heuristic_v2_success_threshold: 2\n", &[]).unwrap_err();
    assert!(
        error.contains(
            "model_list.0.litellm_params.complexity_router_config.heuristic_v2_success_threshold"
        ),
        "got {error}"
    );
}

#[test]
fn the_artifact_name_and_threshold_resolve_from_the_environment() {
    let fields = "        heuristic_v2_artifact: os.environ/TIER_ARTIFACT
        heuristic_v2_success_threshold: os.environ/TIER_THRESHOLD
";
    let configured = only(
        fields,
        &[
            ("TIER_ARTIFACT", "ultrafeedback"),
            ("TIER_THRESHOLD", "0.625"),
        ],
    )
    .unwrap();
    assert_eq!(
        configured,
        Some(Config {
            artifact: ArtifactSource::BuiltIn("ultrafeedback".to_owned()),
            success_threshold: Some(0.625_f64),
        })
    );
}

#[test]
fn an_environment_artifact_name_is_checked_too() {
    let fields = "        heuristic_v2_artifact: os.environ/TIER_ARTIFACT\n";
    let error = only(fields, &[("TIER_ARTIFACT", "unknown")]).unwrap_err();
    assert!(error.contains("invalid scalar"), "got {error}");
}

#[rstest]
#[case::artifact_name("        heuristic_v2_artifact: os.environ/UNSET_ARTIFACT\n")]
#[case::threshold("        heuristic_v2_success_threshold: os.environ/UNSET_THRESHOLD\n")]
fn a_missing_environment_names_the_variable(#[case] fields: &str) {
    let error = only(fields, &[]).unwrap_err();
    assert!(error.contains("is not set"), "got {error}");
    assert!(error.contains("UNSET_"), "got {error}");
}

#[test]
fn an_empty_environment_threshold_leaves_the_key_unset() {
    let fields = "        heuristic_v2_success_threshold: os.environ/TIER_THRESHOLD\n";
    let configured = only(fields, &[("TIER_THRESHOLD", "")]).unwrap().unwrap();
    assert_eq!(configured.success_threshold, None);
}

#[test]
fn a_router_sibling_key_is_ignored_by_this_section() {
    let fields = "        classifier_type: heuristic_v2
        heuristic_v2_artifact: ultrafeedback
";
    let configured = only(fields, &[]).unwrap().unwrap();
    assert_eq!(
        configured.artifact,
        ArtifactSource::BuiltIn("ultrafeedback".to_owned())
    );
}

#[rstest]
#[case::below(-0.1_f64)]
#[case::above(1.1_f64)]
fn config_validation_refuses_a_threshold_outside_the_unit_interval(#[case] threshold: f64) {
    let config = Config {
        artifact: ArtifactSource::default(),
        success_threshold: Some(threshold),
    };
    let error = config
        .validate("heuristic_v2_success_threshold")
        .unwrap_err();
    assert!(error.to_string().contains("invalid scalar"), "got {error}");
}

#[test]
fn config_validation_accepts_the_declared_thresholds() {
    for threshold in [None, Some(0.0_f64), Some(0.5_f64), Some(1.0_f64)] {
        let config = Config {
            artifact: ArtifactSource::default(),
            success_threshold: threshold,
        };
        assert!(config.validate("heuristic_v2_success_threshold").is_ok());
    }
}

#[test]
fn the_shipped_valid_seed_loads() {
    let source =
        include_str!("../../../fuzz/corpus/config_yaml/tier-artifact-predictor-valid.yaml");
    let loaded = settings(source, &[]).unwrap();
    let Some(configured) = loaded.deployments.first().and_then(Option::as_ref) else {
        panic!("the shipped seed configures the tier artifact predictor");
    };
    assert_eq!(
        configured.artifact,
        ArtifactSource::BuiltIn("ultrafeedback".to_owned())
    );
    assert_eq!(
        configured.success_threshold.map(f64::to_bits),
        Some(0.875_f64.to_bits())
    );
}

#[test]
fn the_shipped_invalid_seed_is_refused() {
    let source =
        include_str!("../../../fuzz/corpus/config_yaml/tier-artifact-predictor-invalid.yaml");
    let error = settings(source, &[]).unwrap_err();
    assert!(!error.is_empty());
}
