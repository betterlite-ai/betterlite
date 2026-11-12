const LITELLM_METADATA_NEEDLES: [&str; 5] = [
    "batch",
    "generic_api_call",
    "_acreate_batch",
    "file",
    "_ageneric_api_call_with_fallbacks",
];

// the thread and assistant endpoints, and every batch or file call, name the field litellm_metadata
pub(super) fn router_metadata_variable_name(function_name: Option<&str>) -> &'static str {
    match function_name {
        Some(name)
            if LITELLM_METADATA_NEEDLES
                .iter()
                .any(|method| name.contains(method)) =>
        {
            "litellm_metadata"
        }
        _ => "metadata",
    }
}
