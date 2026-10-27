use std::vec::IntoIter;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::concurrent_conflict::conflicting_paths;
use super::error::{ContentBlocked, ScanTarget};
use super::leaf_rewrite::{leaf_replacements, rewrite_leaves};
use super::process_input::{InputScan, argument_leaves};
use super::text_count_mismatch::masked_texts;
use super::tool_request::McpToolRequest;

mod tests;

pub fn write_back(
    scan: &InputScan,
    request: &mut McpToolRequest,
    returned: Option<Vec<String>>,
) -> Result<(), ContentBlocked> {
    let Some(masked) = masked_texts(returned, scan.inputs.texts.len(), ScanTarget::ToolCall)?
    else {
        return Ok(());
    };
    let mut remaining = masked.into_iter();
    write_tool_definition(scan, request, &mut remaining);
    let replacements = leaf_replacements(&scan.argument_leaves, remaining);
    if replacements.is_empty() {
        return Ok(());
    }
    let current = request.tool_arguments();
    let conflicts = conflicting_paths(
        &scan.argument_leaves,
        &argument_leaves(current)?,
        &replacements,
    );
    if !conflicts.is_empty() {
        return Err(ContentBlocked::ConflictingRewrite { paths: conflicts });
    }
    let mut arguments = current.cloned().unwrap_or(GuardrailValue::Null);
    rewrite_leaves(&mut arguments, &replacements);
    request.mcp_arguments = Some(arguments.clone());
    request.modified_arguments = Some(arguments);
    Ok(())
}

fn write_tool_definition(
    scan: &InputScan,
    request: &mut McpToolRequest,
    masked: &mut IntoIter<String>,
) {
    if let Some(original) = &scan.description
        && let Some(replacement) = masked.next()
        && replacement != *original
    {
        request.mcp_tool_description = Some(replacement);
    }
    let replacements = leaf_replacements(&scan.schema_leaves, masked.by_ref());
    if !replacements.is_empty() {
        let mut schema = GuardrailValue::Object(scan.schema.clone());
        rewrite_leaves(&mut schema, &replacements);
        request.mcp_input_schema = Some(schema);
    }
}
