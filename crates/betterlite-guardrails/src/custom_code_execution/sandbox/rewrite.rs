use super::error::CompileError;
use std::cmp::Reverse;
use tree_sitter::{Language, Node, Parser, Tree};
mod checks;
mod rules;
use rules::definitions::definition_rules;
use rules::{identifier_rules, statement_rules};
const MAX_DEPTH: usize = 200;
const BUDGET_GUARD: &str = "_budget_ok_() and (";
const ITER_GUARD: &str = "_getiter_((";
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Scope {
    pub function_depth: usize,
    pub loop_depth: usize,
    pub is_async: bool,
    pub async_generator: bool,
    pub comprehension_depth: usize,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Function {
    pub name: String,
    pub is_async: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edit {
    pub position: usize,
    pub text: String,
}
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Analysis {
    pub edits: Vec<Edit>,
    pub functions: Vec<Function>,
}
pub fn parse(source: &str, filename: &str) -> Result<Tree, CompileError> {
    let mut parser = Parser::new();
    let language = Language::from(tree_sitter_python::LANGUAGE);
    parser
        .set_language(&language)
        .map_err(|error| CompileError::Grammar { source: error })?;
    parser
        .parse(source, None)
        .ok_or_else(|| CompileError::syntax(filename, 1, "the Python parser produced no tree"))
}
pub fn analyze(tree: &Tree, source: &str, filename: &str) -> Result<Analysis, CompileError> {
    let root = tree.root_node();
    if root.has_error() {
        let broken = first_error(root).unwrap_or(root);
        return Err(CompileError::syntax(
            filename,
            u32::try_from(broken.start_position().row.saturating_add(1)).unwrap_or(u32::MAX),
            "invalid syntax",
        ));
    }
    let mut analysis = Analysis::default();
    walk(root, (source, filename), 0, Scope::default(), &mut analysis)?;
    Ok(analysis)
}
pub(super) fn first_error(root: Node<'_>) -> Option<Node<'_>> {
    let mut stack = vec![(root, 0_usize)];
    while let Some((node, depth)) = stack.pop() {
        if node.is_error() || node.is_missing() {
            return Some(node);
        }
        if depth >= MAX_DEPTH {
            continue;
        }
        for index in (0..node.child_count()).rev() {
            if let Some(child) = node.child(index)
                && child.has_error()
            {
                stack.push((child, depth.saturating_add(1)));
            }
        }
    }
    None
}
fn check_node(
    node: Node<'_>,
    source: &str,
    filename: &str,
    scope: Scope,
    analysis: &mut Analysis,
) -> Result<(), CompileError> {
    let line = u32::try_from(node.start_position().row.saturating_add(1)).unwrap_or(u32::MAX);
    if let Some(reason) = forbidden(node.kind()) {
        return Err(CompileError::forbidden(filename, line, reason.to_owned()));
    }
    statement_rules(node, filename, line, scope)?;
    if node.kind() == "attribute"
        && let Some(attribute) = node.child_by_field_name("attribute")
    {
        rules::check_attribute(attribute, source, filename, line)?;
    }
    if node.kind() == "call"
        && let Some(function) = node.child_by_field_name("function")
    {
        rules::check_called(function, source, filename, line)?;
    }
    identifier_rules(node, source, filename, line)?;
    definition_rules(node, source, filename, scope, analysis)?;
    if matches!(
        node.kind(),
        "while_statement" | "for_statement" | "for_in_clause"
    ) {
        loop_guard(node, analysis);
    }
    Ok(())
}
fn forbidden(kind: &str) -> Option<&'static str> {
    match kind {
        "class_definition" => Some("class definitions are not allowed in the sandbox"),
        "import_statement" | "import_from_statement" => {
            Some("import statements are not allowed in the sandbox")
        }
        "nonlocal_statement" => Some("nonlocal statements are not allowed in the sandbox"),
        "print_statement" => Some("Python 2 print statements are not allowed in the sandbox"),
        "exec_statement" => Some("Exec calls are not allowed."),
        "type_alias_statement" => Some("TypeAlias statements are not allowed."),
        "type_parameter" => Some("TypeVar statements are not allowed."),
        _ => None,
    }
}
fn child_scope(node: Node<'_>, child: Node<'_>, scope: Scope) -> Scope {
    match node.kind() {
        "function_definition" | "lambda" => entering_function(node, scope),
        "for_statement" | "while_statement" if child.kind() != "else_clause" => Scope {
            loop_depth: scope.loop_depth.saturating_add(1),
            ..scope
        },
        "list_comprehension"
        | "set_comprehension"
        | "dictionary_comprehension"
        | "generator_expression" => Scope {
            comprehension_depth: scope.comprehension_depth.saturating_add(1),
            ..scope
        },
        "for_in_clause" if leftmost_iterable(node, child) => Scope {
            comprehension_depth: scope.comprehension_depth.saturating_sub(1),
            ..scope
        },
        _ => scope,
    }
}
fn entering_function(node: Node<'_>, scope: Scope) -> Scope {
    let mut cursor = node.walk();
    let is_async = node.kind() == "function_definition"
        && node
            .children(&mut cursor)
            .any(|child| child.kind() == "async");
    let async_generator = is_async && node.child_by_field_name("body").is_some_and(has_yield);
    Scope {
        function_depth: scope.function_depth.saturating_add(1),
        loop_depth: 0,
        is_async,
        async_generator,
        comprehension_depth: 0,
    }
}
fn has_yield(node: Node<'_>) -> bool {
    let mut stack = vec![node];
    while let Some(current) = stack.pop() {
        if current.kind() == "yield" {
            return true;
        }
        if matches!(
            current.kind(),
            "function_definition" | "lambda" | "class_definition"
        ) {
            continue;
        }
        let mut cursor = current.walk();
        stack.extend(current.named_children(&mut cursor));
    }
    false
}
fn leftmost_iterable(node: Node<'_>, child: Node<'_>) -> bool {
    if node
        .child_by_field_name("right")
        .is_none_or(|right| right.id() != child.id())
    {
        return false;
    }
    let Some(parent) = node.parent() else {
        return false;
    };
    if !matches!(
        parent.kind(),
        "list_comprehension"
            | "set_comprehension"
            | "dictionary_comprehension"
            | "generator_expression"
    ) {
        return false;
    }
    let mut cursor = parent.walk();
    parent
        .named_children(&mut cursor)
        .find(|sibling| sibling.kind() == "for_in_clause")
        .is_some_and(|first| first.id() == node.id())
}
fn loop_guard(node: Node<'_>, analysis: &mut Analysis) {
    if let Some(condition) = node.child_by_field_name("condition") {
        analysis.edits.push(Edit {
            position: condition.start_byte(),
            text: BUDGET_GUARD.to_owned(),
        });
        analysis.edits.push(Edit {
            position: condition.end_byte(),
            text: ")".to_owned(),
        });
    }
    let mut cursor = node.walk();
    let is_async = node
        .children(&mut cursor)
        .any(|child| child.kind() == "async");
    let guarded = node.kind() == "for_in_clause" || (node.kind() == "for_statement" && !is_async);
    if !guarded {
        return;
    }
    if let Some(iterable) = node.child_by_field_name("right") {
        analysis.edits.push(Edit {
            position: iterable.start_byte(),
            text: ITER_GUARD.to_owned(),
        });
        analysis.edits.push(Edit {
            position: iterable.end_byte(),
            text: "))".to_owned(),
        });
    }
}
fn walk(
    node: Node<'_>,
    location: (&str, &str),
    depth: usize,
    scope: Scope,
    analysis: &mut Analysis,
) -> Result<(), CompileError> {
    let (source, filename) = location;
    let line = u32::try_from(node.start_position().row.saturating_add(1)).unwrap_or(u32::MAX);
    if depth > MAX_DEPTH {
        return Err(CompileError::syntax(
            filename,
            line,
            "the program nests too deeply",
        ));
    }
    check_node(node, source, filename, scope, analysis)?;
    let mut cursor = node.walk();
    let children: Vec<Node<'_>> = node.named_children(&mut cursor).collect();
    for child in children {
        walk(
            child,
            location,
            depth.saturating_add(1),
            child_scope(node, child, scope),
            analysis,
        )?;
    }
    Ok(())
}
pub fn apply(source: &str, edits: &[Edit], filename: &str) -> Result<String, CompileError> {
    let mut ordered: Vec<&Edit> = edits.iter().collect();
    ordered.sort_by_key(|edit| Reverse(edit.position));
    let mut rendered = source.to_owned();
    for edit in ordered {
        if edit.position > rendered.len() || !rendered.is_char_boundary(edit.position) {
            return Err(CompileError::syntax(
                filename,
                1,
                "a rewrite position is not a character boundary",
            ));
        }
        rendered.insert_str(edit.position, &edit.text);
    }
    Ok(rendered)
}
