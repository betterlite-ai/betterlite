use super::super::checks::{NameKind, check_name};
use super::super::{Analysis, Function, Scope};
use crate::custom_code_execution::sandbox::error::CompileError;
use tree_sitter::Node;
use unicode_normalization::UnicodeNormalization as _;
pub(crate) fn definition_rules(
    node: Node<'_>,
    source: &str,
    filename: &str,
    scope: Scope,
    analysis: &mut Analysis,
) -> Result<(), CompileError> {
    let line = u32::try_from(node.start_position().row.saturating_add(1)).unwrap_or(u32::MAX);
    if node.kind() == "argument_list" {
        return arguments(node, filename, line);
    }
    if node.kind() != "function_definition" {
        return Ok(());
    }
    let name = node.child_by_field_name("name").map_or("", |field| {
        source.get(field.byte_range()).unwrap_or_default()
    });
    let kind = if node.start_position().column > 0 {
        NameKind::Method
    } else {
        NameKind::Variable
    };
    check_name(&name.nfkc().collect::<String>(), filename, line, kind)?;
    if let Some(parameters) = node.child_by_field_name("parameters") {
        parameters_rule(parameters, source, filename, line)?;
    }
    if scope.function_depth == 0 {
        analysis.functions.retain(|function| function.name != name);
        let mut cursor = node.walk();
        let is_async = node
            .children(&mut cursor)
            .any(|child| child.kind() == "async");
        analysis.functions.push(Function {
            name: name.nfkc().collect::<String>(),
            is_async,
        });
    }
    Ok(())
}

fn parameters_rule(
    node: Node<'_>,
    source: &str,
    filename: &str,
    line: u32,
) -> Result<(), CompileError> {
    let mut seen: Vec<String> = Vec::new();
    let mut defaulted = false;
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        let (found, has_default) = parameter(child, source);
        let Some(parameter_name) = found else {
            continue;
        };
        let normalized_name: String = parameter_name.nfkc().collect();
        if seen.contains(&normalized_name) {
            return Err(CompileError::syntax(
                filename,
                line,
                "duplicate parameter name",
            ));
        }
        if defaulted && !has_default {
            return Err(CompileError::syntax(
                filename,
                line,
                "non-default parameter after a default",
            ));
        }
        defaulted = defaulted || has_default;
        seen.push(normalized_name);
    }
    Ok(())
}
fn parameter<'source>(node: Node<'_>, source: &'source str) -> (Option<&'source str>, bool) {
    let field = match node.kind() {
        "identifier" => Some((node, false)),
        "default_parameter" | "typed_default_parameter" => {
            node.child_by_field_name("name").map(|name| (name, true))
        }
        "typed_parameter" => node.child_by_field_name("name").map(|name| (name, false)),
        "list_splat_pattern" | "dictionary_splat_pattern" => {
            node.named_child(0).map(|name| (name, false))
        }
        _ => None,
    };
    match field {
        Some((name, defaulted)) => (
            Some(source.get(name.byte_range()).unwrap_or_default()),
            defaulted,
        ),
        None => (None, false),
    }
}
fn arguments(node: Node<'_>, filename: &str, line: u32) -> Result<(), CompileError> {
    let mut keyword_seen = false;
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "keyword_argument" => keyword_seen = true,
            "list_splat" | "dictionary_splat" | "generator_expression" => {}
            _ if keyword_seen => {
                return Err(CompileError::syntax(
                    filename,
                    line,
                    "positional after a keyword argument",
                ));
            }
            _ => {}
        }
    }
    Ok(())
}
