use tree_sitter::Node;

use super::super::super::sandbox::error::CompileError;
use super::checks::{NameKind, check_call, check_name};
use unicode_normalization::UnicodeNormalization as _;

use super::{Analysis, Function, Scope};

pub(super) fn statement_rules(
    node: Node<'_>,
    filename: &str,
    line: u32,
    scope: Scope,
) -> Result<(), CompileError> {
    comprehension_iterable(node, filename, line)?;
    match node.kind() {
        "return_statement" if scope.function_depth == 0 => Err(CompileError::syntax(
            filename,
            line,
            "return outside function",
        )),
        "break_statement" | "continue_statement" if scope.loop_depth == 0 => Err(
            CompileError::syntax(filename, line, "loop control outside loop"),
        ),
        "yield" if scope.function_depth == 0 => Err(CompileError::syntax(
            filename,
            line,
            "yield outside function",
        )),
        "await" if !scope.is_async => Err(CompileError::syntax(
            filename,
            line,
            "await outside async function",
        )),

        "assignment" if node.child_by_field_name("type").is_some() => Err(CompileError::forbidden(
            filename,
            line,
            "annotated assignment is not allowed in the sandbox".to_owned(),
        )),
        "augmented_assignment" => augmented_target(node, filename, line),
        "match_statement" => Err(CompileError::forbidden(
            filename,
            line,
            "match statements are not allowed in the sandbox".to_owned(),
        )),
        "except_clause" if contains_token(node, "*") => Err(CompileError::forbidden(
            filename,
            line,
            "except* is not allowed in the sandbox".to_owned(),
        )),
        _ => Ok(()),
    }
}

fn comprehension_iterable(node: Node<'_>, filename: &str, line: u32) -> Result<(), CompileError> {
    if node.kind() != "for_in_clause" {
        return Ok(());
    }
    let mut cursor = node.walk();
    if node.children_by_field_name("right", &mut cursor).count() <= 1 {
        return Ok(());
    }
    Err(CompileError::syntax(
        filename,
        line,
        "a comprehension takes a single iterable",
    ))
}

fn augmented_target(node: Node<'_>, filename: &str, line: u32) -> Result<(), CompileError> {
    let Some(left) = node.child_by_field_name("left") else {
        return Ok(());
    };
    match left.kind() {
        "attribute" => Err(CompileError::forbidden(
            filename,
            line,
            "Augmented assignment of attributes is not allowed.".to_owned(),
        )),
        "subscript" => Err(CompileError::forbidden(
            filename,
            line,
            "Augmented assignment of object items and slices is not allowed.".to_owned(),
        )),
        _ => Ok(()),
    }
}

pub(super) fn expression_rules(
    node: Node<'_>,
    source: &str,
    filename: &str,
    line: u32,
) -> Result<(), CompileError> {
    match node.kind() {
        "attribute" => attribute(node, source, filename, line),
        "call" => called(node, source, filename, line),
        "identifier" => identifier(node, source, filename, line),
        "comparison_operator" if contains_token(node, "<>") => Err(CompileError::syntax(
            filename,
            line,
            "the Python 2 '<>' comparison is not allowed",
        )),
        "list" | "set" | "dictionary" if mixed_comprehension(node) => Err(CompileError::syntax(
            filename,
            line,
            "a comprehension takes a single iterable",
        )),
        _ => Ok(()),
    }
}

fn contains_token(node: Node<'_>, token: &str) -> bool {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .any(|child| child.kind() == token)
}

// A display mixing a comprehension with another element is what the same source falls back to.
fn mixed_comprehension(node: Node<'_>) -> bool {
    let mut cursor = node.walk();
    let children: Vec<Node<'_>> = node.named_children(&mut cursor).collect();
    children.len() > 1
        && children.iter().any(|child| {
            matches!(
                child.kind(),
                "list_comprehension" | "set_comprehension" | "dictionary_comprehension"
            )
        })
}

fn attribute(node: Node<'_>, source: &str, filename: &str, line: u32) -> Result<(), CompileError> {
    let Some(attribute) = node.child_by_field_name("attribute") else {
        return Ok(());
    };
    check_name(
        &source
            .get(attribute.byte_range())
            .unwrap_or_default()
            .nfkc()
            .collect::<String>(),
        filename,
        line,
        NameKind::Attribute,
    )
}

fn called(node: Node<'_>, source: &str, filename: &str, line: u32) -> Result<(), CompileError> {
    let Some(function) = node.child_by_field_name("function") else {
        return Ok(());
    };
    let target = unwrap_parenthesized(function);
    if target.kind() != "identifier" {
        return Ok(());
    }
    check_call(
        &source
            .get(target.byte_range())
            .unwrap_or_default()
            .nfkc()
            .collect::<String>(),
        filename,
        line,
    )
}

pub(super) fn unwrap_parenthesized(node: Node<'_>) -> Node<'_> {
    if node.kind() != "parenthesized_expression" {
        return node;
    }
    node.named_child(0).map_or(node, unwrap_parenthesized)
}

// A name in an attribute's own slot, a keyword argument's name and a definition's name has its own rule.
fn identifier(node: Node<'_>, source: &str, filename: &str, line: u32) -> Result<(), CompileError> {
    let Some(parent) = node.parent() else {
        return check_name(
            &source
                .get(node.byte_range())
                .unwrap_or_default()
                .nfkc()
                .collect::<String>(),
            filename,
            line,
            NameKind::Variable,
        );
    };
    let slot = match parent.kind() {
        "attribute" => "attribute",
        "keyword_argument" | "function_definition" => "name",
        _ => "",
    };
    if !slot.is_empty()
        && parent
            .child_by_field_name(slot)
            .is_some_and(|field| field.id() == node.id())
    {
        return Ok(());
    }
    check_name(
        &source
            .get(node.byte_range())
            .unwrap_or_default()
            .nfkc()
            .collect::<String>(),
        filename,
        line,
        NameKind::Variable,
    )
}

pub(super) fn definition_rules(
    node: Node<'_>,
    source: &str,
    filename: &str,
    scope: Scope,
    analysis: &mut Analysis,
) -> Result<(), CompileError> {
    let line = u32::try_from(node.start_position().row.saturating_add(1)).unwrap_or(u32::MAX);
    match node.kind() {
        "function_definition" => definition(node, source, filename, scope, analysis),
        "argument_list" => arguments(node, filename, line),
        _ => Ok(()),
    }
}

fn definition(
    node: Node<'_>,
    source: &str,
    filename: &str,
    scope: Scope,
    analysis: &mut Analysis,
) -> Result<(), CompileError> {
    let line = u32::try_from(node.start_position().row.saturating_add(1)).unwrap_or(u32::MAX);
    let name = node.child_by_field_name("name").map_or("", |field| {
        source.get(field.byte_range()).unwrap_or_default()
    });
    let nested = node.start_position().column > 0;
    let kind = if nested {
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
        analysis.functions.push(Function {
            name: name.nfkc().collect::<String>(),
            is_async: node
                .children(&mut cursor)
                .any(|child| child.kind() == "async"),
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
                    "positional argument after a keyword argument",
                ));
            }
            _ => {}
        }
    }
    Ok(())
}
