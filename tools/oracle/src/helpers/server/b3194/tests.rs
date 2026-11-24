#![cfg(test)]

use std::fs;
use std::path::Path;

use betterlite_oracle_runner::case::helper::HelperCase;
use betterlite_oracle_runner::helpers::{HelperRun, ORACLE_DIR, run_case};
use rstest::rstest;

#[rstest]
#[case::agent_empty("public/resource-hubs/agent-hub-empty-when-disabled")]
#[case::agent_filtered("public/resource-hubs/agent-hub-filters-by-allowlist")]
#[case::agent_url("public/resource-hubs/agent-hub-includes-public-url")]
#[case::agent_isdisjoint("public/resource-hubs/agent-hub-isdisjoint-check")]
#[case::mcp_empty("public/resource-hubs/mcp-hub-empty-when-disabled")]
#[case::mcp_filtered("public/resource-hubs/mcp-hub-filters-by-allowlist")]
#[case::mcp_overrides("public/resource-hubs/mcp-hub-overrides-is-public-fields")]
#[case::skill_basic("public/resource-hubs/skill-hub-basic-manifest-parsing")]
#[case::skill_missing("public/resource-hubs/skill-hub-missing-manifest-fields")]
#[case::skill_null("public/resource-hubs/skill-hub-null-manifest")]
#[case::skill_parse_error("public/resource-hubs/skill-hub-manifest-parse-error-500")]
#[case::skill_database_error("public/resource-hubs/skill-hub-database-error-500")]
#[case::skill_shape("public/resource-hubs/skill-hub-count-and-response-shape")]
#[case::skill_timestamps("public/resource-hubs/skill-hub-timestamps-stringified")]
fn replays_b3194_public_hubs_against_litellm(#[case] name: &str) {
    let root = Path::new(ORACLE_DIR);
    let source = fs::read_to_string(root.join(format!("cases/{name}.yaml"))).unwrap();
    let case = serde_saphyr::from_str::<HelperCase>(&source).unwrap();
    let result = run_case(&[crate::GROUP], &case, false).unwrap();
    assert!(matches!(result, HelperRun::Match));
}
