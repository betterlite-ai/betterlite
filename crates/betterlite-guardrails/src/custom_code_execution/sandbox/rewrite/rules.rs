use tree_sitter::Node;

use super::super::super::sandbox::error::CompileError;
use super::checks::{NameKind, check_call, check_name};
use unicode_normalization::UnicodeNormalization as _;

use super::Scope;

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

pub(crate) mod definitions;
