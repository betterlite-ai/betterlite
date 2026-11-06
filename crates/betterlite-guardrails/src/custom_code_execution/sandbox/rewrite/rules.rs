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
            "annotated assignment is not allowed".to_owned(),
        )),
        "augmented_assignment" => augmented_target(node, filename, line),
        "match_statement" => Err(CompileError::forbidden(
            filename,
            line,
            "match statements are not allowed".to_owned(),
        )),
        "except_clause" if contains_token(node, "*") => Err(CompileError::forbidden(
            filename,
            line,
            "except* is not allowed".to_owned(),
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
            "Augmented assignment of object items is not allowed.".to_owned(),
        )),
        _ => Ok(()),
    }
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
    let target = unwrap_parenthesized(function);
    if target.kind() != "identifier" {
        return Ok(());
    }
    check_call(&name_text(source, target), filename, line)
}
pub(super) fn syntax_rules(node: Node<'_>, filename: &str, line: u32) -> Result<(), CompileError> {
    if node.kind() == "comparison_operator" && contains_token(node, "<>") {
        return Err(CompileError::syntax(
            filename,
            line,
            "the Python 2 '<>' comparison is not allowed",
        ));
    }
    if matches!(node.kind(), "list" | "set" | "dictionary") && mixed_comprehension(node) {
        return Err(CompileError::syntax(
            filename,
            line,
            "a comprehension takes a single iterable",
        ));
    }
    Ok(())
}
pub(super) fn identifier_rules(
    node: Node<'_>,
    source: &str,
    filename: &str,
    line: u32,
) -> Result<(), CompileError> {
    let Some(parent) = node.parent() else {
        return check_name(&name_text(source, node), filename, line, NameKind::Variable);
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
    check_name(&name_text(source, node), filename, line, NameKind::Variable)
}
fn contains_token(node: Node<'_>, token: &str) -> bool {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .any(|child| child.kind() == token)
}
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
fn name_text(source: &str, node: Node<'_>) -> String {
    source
        .get(node.byte_range())
        .map_or_else(String::new, |text| text.nfkc().collect())
}
pub(super) fn unwrap_parenthesized(node: Node<'_>) -> Node<'_> {
    if node.kind() != "parenthesized_expression" {
        return node;
    }
    node.named_child(0).map_or(node, unwrap_parenthesized)
}
pub(crate) mod definitions;
