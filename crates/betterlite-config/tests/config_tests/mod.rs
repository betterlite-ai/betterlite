#![cfg(test)]
#![expect(
    clippy::disallowed_methods,
    reason = "insta uses Into internally to normalize its snapshot name"
)]

use std::collections::BTreeMap;
use std::fs::{read, read_dir, read_to_string};
use std::io::Error;
use std::str::from_utf8;

use betterlite_config::loader::ConfigLoader;

macro_rules! fixture_test {
    ($test:ident, $fixture:literal, $snapshot:literal) => {
        #[test]
        fn $test() {
            let path = format!("tests/fixtures/litellm/{}", $fixture);
            let source = read_to_string(path).unwrap();
            let result = ConfigLoader {
                lookup: BTreeMap::<String, String>::new(),
            }
            .load(&source);

            insta::assert_debug_snapshot!($snapshot, result);
        }
    };
}

fixture_test!(
    adaptive_router_example,
    "adaptive_router_example.yaml",
    "adaptive_router_example_yaml"
);
fixture_test!(
    agentcore_websearch_config,
    "agentcore_websearch_config.yaml",
    "agentcore_websearch_config_yaml"
);
fixture_test!(aliases_config, "aliases_config.yaml", "aliases_config_yaml");
fixture_test!(azure_config, "azure_config.yaml", "azure_config_yaml");
fixture_test!(
    bing_grounding_websearch_config,
    "bing_grounding_websearch_config.yaml",
    "bing_grounding_websearch_config_yaml"
);
fixture_test!(
    code_interpreter_interception_config,
    "code_interpreter_interception_config.yaml",
    "code_interpreter_interception_config_yaml"
);
fixture_test!(
    disable_schema_update,
    "disable_schema_update.yaml",
    "disable_schema_update_yaml"
);
fixture_test!(
    enterprise_config,
    "enterprise_config.yaml",
    "enterprise_config_yaml"
);
fixture_test!(
    health_check_test_config,
    "_health_check_test_config.yaml",
    "_health_check_test_config_yaml"
);
fixture_test!(
    langfuse_config,
    "langfuse_config.yaml",
    "langfuse_config_yaml"
);
fixture_test!(load_balancer, "load_balancer.yaml", "load_balancer_yaml");
fixture_test!(
    multi_instance_simple_config,
    "multi_instance_simple_config.yaml",
    "multi_instance_simple_config_yaml"
);
fixture_test!(
    oai_misc_config,
    "oai_misc_config.yaml",
    "oai_misc_config_yaml"
);
fixture_test!(
    opentelemetry_config,
    "opentelemetry_config.yaml",
    "opentelemetry_config_yaml"
);
fixture_test!(
    otel_test_config,
    "otel_test_config.yaml",
    "otel_test_config_yaml"
);
fixture_test!(
    pass_through_config,
    "pass_through_config.yaml",
    "pass_through_config_yaml"
);
fixture_test!(
    reject_clientside_metadata_tags_config,
    "reject_clientside_metadata_tags_config.yaml",
    "reject_clientside_metadata_tags_config_yaml"
);
fixture_test!(simple_config, "simple_config.yaml", "simple_config_yaml");
fixture_test!(
    spend_tracking_config,
    "spend_tracking_config.yaml",
    "spend_tracking_config_yaml"
);
fixture_test!(
    store_model_db_config,
    "store_model_db_config.yaml",
    "store_model_db_config_yaml"
);
fixture_test!(
    pipeline_config,
    "test_pipeline_config.yaml",
    "test_pipeline_config_yaml"
);
fixture_test!(
    tool_permission_example,
    "tool_permission_example.yaml",
    "tool_permission_example_yaml"
);
fixture_test!(
    websearch_interception_config,
    "websearch_interception_config.yaml",
    "websearch_interception_config_yaml"
);

#[test]
fn replays_the_config_yaml_corpus() {
    read_dir("fuzz/corpus/config_yaml")
        .unwrap()
        .try_for_each(|entry_result| {
            let entry = entry_result?;
            let bytes = read(entry.path())?;
            if let Ok(source) = from_utf8(&bytes) {
                drop(
                    ConfigLoader {
                        lookup: BTreeMap::<String, String>::new(),
                    }
                    .load(source),
                );
            }
            Ok::<(), Error>(())
        })
        .unwrap();
}
