use std::collections::HashSet;

mod tests;

// Keeps every request tool and appends a returned tool only when no request tool and no earlier returned tool has its name.
#[must_use]
pub fn merge_returned_tools<Tool, NameOf>(
    request_tools: &[Tool],
    returned_tools: &[Tool],
    tool_name: NameOf,
) -> Vec<Tool>
where
    Tool: Clone,
    NameOf: Fn(&Tool) -> Option<String>,
{
    let mut taken_names: HashSet<String> = request_tools.iter().filter_map(&tool_name).collect();
    let mut merged = request_tools.to_vec();
    for tool in returned_tools {
        let Some(name) = tool_name(tool) else {
            merged.push(tool.clone());
            continue;
        };
        if taken_names.insert(name) {
            merged.push(tool.clone());
        }
    }
    merged
}
