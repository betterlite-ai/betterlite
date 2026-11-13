use super::super::super::sandbox::error::CompileError;
use super::Scope;
use super::checks::{NameKind, check_call, check_name};
use tree_sitter::Node;
use unicode_normalization::UnicodeNormalization as _;
pub(super) fn statement_rules(
    node: Node<'_>,
    filename: &str,
    line: u32,
    scope: Scope,
) -> Result<(), CompileError> {
    let violation = match node.kind() {
        "for_in_clause" if contains_token(node, ",") => {
            Some("a comprehension takes a single iterable")
        }
        "comparison_operator" if contains_token(node, "<>") => {
            Some("the Python 2 '<>' comparison is not allowed")
        }
        _ => None,
    };
    if let Some(message) = violation {
        return Err(CompileError::syntax(filename, line, message));
    }
    async_rules(node, filename, line, scope)?;
    flow_rules(node, filename, line, scope)?;
    syntax_kind_rules(node, filename, line)
}
fn async_rules(
    node: Node<'_>,
    filename: &str,
    line: u32,
    scope: Scope,
) -> Result<(), CompileError> {
    let asynchronous = contains_token(node, "async") && !scope.is_async;
    if matches!(node.kind(), "for_statement" | "with_statement") && asynchronous {
        return Err(CompileError::syntax(
            filename,
            line,
            "async code outside async function",
        ));
    }
    if node.kind() == "for_in_clause" && asynchronous {
        return Err(CompileError::syntax(
            filename,
            line,
            "asynchronous comprehension outside an async function",
        ));
    }
    if node.kind() == "await" && !scope.is_async {
        return Err(CompileError::syntax(
            filename,
            line,
            "await outside async function",
        ));
    }
    Ok(())
}
fn flow_rules(node: Node<'_>, filename: &str, line: u32, scope: Scope) -> Result<(), CompileError> {
    match node.kind() {
        "yield" if scope.comprehension_depth > 0 => Err(CompileError::syntax(
            filename,
            line,
            "yield inside a comprehension",
        )),
        "yield" if contains_token(node, "from") && scope.is_async => Err(CompileError::syntax(
            filename,
            line,
            "yield from inside async function",
        )),
        "yield" if scope.function_depth == 0 => Err(CompileError::syntax(
            filename,
            line,
            "yield outside function",
        )),
        "return_statement" if scope.is_async && scope.async_generator && has_return_value(node) => {
            Err(CompileError::syntax(
                filename,
                line,
                "return with value in async generator",
            ))
        }
        "return_statement" if scope.function_depth == 0 => Err(CompileError::syntax(
            filename,
            line,
            "return outside function",
        )),
        "break_statement" | "continue_statement" if scope.loop_depth == 0 => Err(
            CompileError::syntax(filename, line, "loop control outside loop"),
        ),
        _ => Ok(()),
    }
}
fn syntax_kind_rules(node: Node<'_>, filename: &str, line: u32) -> Result<(), CompileError> {
    if node.kind() == "augmented_assignment" {
        return augmented_target(node, filename, line);
    }
    let message = match node.kind() {
        "assignment" if node.child_by_field_name("type").is_some() => {
            "annotated assignment is not allowed"
        }
        "function_definition" if node.child_by_field_name("type_parameters").is_some() => {
            "TypeVar statements are not allowed."
        }
        "match_statement" => "match statements are not allowed",
        "except_clause" if contains_token(node, "*") => "except* is not allowed",
        _ => return Ok(()),
    };
    Err(CompileError::forbidden(filename, line, message.to_owned()))
}
fn has_return_value(node: Node<'_>) -> bool {
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .any(|child| child.kind() != "comment")
}
fn augmented_target(node: Node<'_>, filename: &str, line: u32) -> Result<(), CompileError> {
    let Some(left) = node.child_by_field_name("left") else {
        return Ok(());
    };
    if left.kind() == "attribute" {
        return Err(CompileError::forbidden(
            filename,
            line,
            "Augmented assignment of attributes is not allowed.".to_owned(),
        ));
    }
    if left.kind() == "subscript" {
        return Err(CompileError::forbidden(
            filename,
            line,
            "Augmented assignment of object items is not allowed.".to_owned(),
        ));
    }
    Ok(())
}
pub(super) fn check_attribute(
    attribute: Node<'_>,
    source: &str,
    filename: &str,
    line: u32,
) -> Result<(), CompileError> {
    check_name(
        &name_text(source, attribute),
        filename,
        line,
        NameKind::Attribute,
    )
}
pub(super) fn check_called(
    function: Node<'_>,
    source: &str,
    filename: &str,
    line: u32,
) -> Result<(), CompileError> {
    let target = unwrap_parenthesized(function, filename, line)?;
    if target.kind() != "identifier" {
        return Ok(());
    }
    check_call(&name_text(source, target), filename, line)
}
pub(super) fn identifier_rules(
    node: Node<'_>,
    source: &str,
    filename: &str,
    line: u32,
) -> Result<(), CompileError> {
    let Some(parent) = node.parent() else {
        return Ok(());
    };
    let slot = match parent.kind() {
        "attribute" => "attribute",
        "keyword_argument" | "function_definition" => "name",
        _ => "",
    };
    let own_slot = !slot.is_empty()
        && parent
            .child_by_field_name(slot)
            .is_some_and(|field| field.id() == node.id());
    if node.kind() != "identifier" || own_slot {
        return Ok(());
    }
    let name = name_text(source, node);
    if !is_binding(node) && matches!(name.as_str(), "print" | "printed") {
        return Ok(());
    }
    check_name(&name, filename, line, NameKind::Variable)
}
fn in_target_list(node: Node<'_>) -> bool {
    let mut current = node.parent();
    while let Some(parent) = current {
        match parent.kind() {
            "as_pattern_target" | "delete_statement" => return true,
            "parenthesized_expression"
            | "expression_list"
            | "tuple"
            | "list"
            | "pattern_list"
            | "tuple_pattern"
            | "list_pattern"
            | "list_splat"
            | "list_splat_pattern" => {
                current = parent.parent();
            }
            _ => return false,
        }
    }
    false
}
fn is_binding(node: Node<'_>) -> bool {
    let Some(parent) = node.parent() else {
        return false;
    };
    if in_target_list(node) {
        return true;
    }
    match parent.kind() {
        "parameters"
        | "lambda_parameters"
        | "list_splat_pattern"
        | "dictionary_splat_pattern"
        | "pattern_list"
        | "tuple_pattern"
        | "list_pattern"
        | "nonlocal_statement" => true,
        "typed_parameter" => parent
            .named_child(0)
            .is_some_and(|first| first.id() == node.id()),
        "default_parameter" | "typed_default_parameter" | "named_expression" => parent
            .child_by_field_name("name")
            .is_some_and(|name| name.id() == node.id()),
        "except_clause" => parent
            .child_by_field_name("alias")
            .is_some_and(|alias| alias.id() == node.id()),
        "assignment" | "augmented_assignment" | "for_statement" | "for_in_clause" => parent
            .child_by_field_name("left")
            .is_some_and(|left| left.id() == node.id()),
        _ => false,
    }
}
fn contains_token(node: Node<'_>, token: &str) -> bool {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .any(|child| child.kind() == token)
}
fn name_text(source: &str, node: Node<'_>) -> String {
    source
        .get(node.byte_range())
        .map_or_else(String::new, |text| text.nfkc().collect())
}
pub(super) fn unwrap_parenthesized<'source>(
    node: Node<'source>,
    filename: &str,
    line: u32,
) -> Result<Node<'source>, CompileError> {
    let mut current = node;
    let mut depth = 0_usize;
    while current.kind() == "parenthesized_expression" {
        depth = depth.saturating_add(1);
        if depth > super::MAX_DEPTH {
            return Err(CompileError::syntax(
                filename,
                line,
                "the program nests too deeply",
            ));
        }
        match current.named_child(0) {
            Some(inner) => current = inner,
            None => break,
        }
    }
    Ok(current)
}
pub(crate) mod definitions;
