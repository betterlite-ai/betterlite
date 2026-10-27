use super::call_tool_result::CallToolResult;
use super::error::ContentBlocked;
use super::leaf_rewrite::{leaf_replacements, rewrite_leaves};
use super::process_output::OutputScan;

mod tests;

// the logging payload captured before this hook holds the same result, so it must change in place
pub fn mask_result(
    scan: &OutputScan,
    result: &mut CallToolResult,
    masked: Vec<String>,
) -> Result<(), ContentBlocked> {
    let mut remaining = masked.into_iter();
    for ((index, original), replacement) in scan.text_blocks.iter().zip(remaining.by_ref()) {
        if replacement != *original
            && let Some(item) = result.content.get_mut(*index)
        {
            item.text = Some(replacement);
        }
    }
    let replacements = leaf_replacements(&scan.structured.leaves, remaining.by_ref());
    if scan
        .structured
        .labels
        .iter()
        .zip(remaining)
        .any(|(original, replacement)| replacement != *original)
    {
        return Err(ContentBlocked::UnrewritableLabel);
    }
    if let Some(structured) = &mut result.structured_content {
        rewrite_leaves(structured, &replacements);
    }
    Ok(())
}
