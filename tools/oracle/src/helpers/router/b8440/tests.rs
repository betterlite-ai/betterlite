#![cfg(test)]

use rstest::rstest;

#[rstest]
#[case::targets("silent-experiment/targets")]
#[case::mirror_kwargs("silent-experiment/mirror-kwargs")]
#[case::excluded_filter("weighted-failover/excluded-filter")]
fn replays_b8440_against_litellm(#[case] name: &str) {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let source = std::fs::read_to_string(root.join(format!("cases/helpers/{name}.yaml"))).unwrap();
    let case = serde_saphyr::from_str::<crate::case::HelperCase>(&source).unwrap();
    let result = crate::helpers::run_case(&case, false).unwrap();
    assert!(matches!(result, crate::helpers::HelperRun::Match));
}
