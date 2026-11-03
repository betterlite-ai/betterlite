#![cfg(test)]

#[test]
fn file_formats_apply_yaml_defaults_and_json_mappings() {
    super::tests::file_yaml_defaults_action_and_json_deduplicates_and_maps_severity();
}
