use std::cmp::Reverse;

use tree_sitter::{Language, Node, Parser, Tree};

use super::error::CompileError;

mod checks;

use checks::{NameKind, check_call, check_name};

const MAX_DEPTH: usize = 200;
const BUDGET_GUARD: &str = "_budget_ok_() and ";
const ITER_GUARD: &str = "_getiter_";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Function {
    pub name: String,
    pub is_async: bool,
}

// A pure insertion into the source, so edits at different offsets never invalidate each other.
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
    parser.set_language(&language).map_err(|_error| {
        CompileError::new(filename, 1, "could not load the Python grammar".to_owned())
    })?;
    parser.parse(source, None).ok_or_else(|| {
        CompileError::new(
            filename,
            1,
            "the Python parser did not produce a tree".to_owned(),
        )
    })
}

pub fn analyze(tree: &Tree, source: &str, filename: &str) -> Result<Analysis, CompileError> {
    let root = tree.root_node();
    if root.has_error() {
        let broken = first_error(root).unwrap_or(root);
        let line = u32::try_from(broken.start_position().row.saturating_add(1)).unwrap_or(u32::MAX);
        return Err(CompileError::new(
            filename,
            line,
            "invalid syntax".to_owned(),
        ));
    }
    let mut analysis = Analysis::default();
    walk(root, source, filename, 0, &mut analysis)?;
    Ok(analysis)
}

fn first_error(node: Node<'_>) -> Option<Node<'_>> {
    if node.is_error() || node.is_missing() {
        return Some(node);
    }
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .filter(Node::has_error)
        .find_map(first_error)
}

fn check_node(
    node: Node<'_>,
    source: &str,
    filename: &str,
    line: u32,
    analysis: &mut Analysis,
) -> Result<(), CompileError> {
    if let Some(reason) = forbidden(node.kind()) {
        return Err(CompileError::new(filename, line, reason.to_owned()));
    }
    if node.kind() == "function_definition" {
        record_function(node, source, filename, line, analysis)?;
    }
    if matches!(
        node.kind(),
        "while_statement" | "for_statement" | "for_in_clause"
    ) {
        loop_guard(node, analysis);
    }
    check_details(node, source, filename, line)
}

fn forbidden(kind: &str) -> Option<&'static str> {
    match kind {
        "class_definition" => Some("class definitions are not allowed in the sandbox"),
        "import_statement" | "import_from_statement" => {
            Some("import statements are not allowed in the sandbox")
        }
        "nonlocal_statement" => Some("nonlocal statements are not allowed in the sandbox"),
        _ => None,
    }
}

fn check_details(
    node: Node<'_>,
    source: &str,
    filename: &str,
    line: u32,
) -> Result<(), CompileError> {
    match node.kind() {
        "attribute" => {
            let field = node.child_by_field_name("attribute");
            field.map_or(Ok(()), |attribute| {
                check_name(
                    source.get(attribute.byte_range()).unwrap_or_default(),
                    filename,
                    line,
                    NameKind::Attribute,
                )
            })
        }
        "call" => {
            let field = node
                .child_by_field_name("function")
                .filter(|function| function.kind() == "identifier");
            field.map_or(Ok(()), |function| {
                check_call(
                    source.get(function.byte_range()).unwrap_or_default(),
                    filename,
                    line,
                )
            })
        }
        "identifier" => {
            let attribute = node
                .parent()
                .is_some_and(|parent| parent.kind() == "attribute");
            if attribute {
                return Ok(());
            }
            check_name(
                source.get(node.byte_range()).unwrap_or_default(),
                filename,
                line,
                NameKind::Variable,
            )
        }
        _ => Ok(()),
    }
}

// A while test is prefixed with `_budget_ok_()` and a for iterable wrapped in `_getiter_`, so a loop stops at the deadline.
fn loop_guard(node: Node<'_>, analysis: &mut Analysis) {
    if let Some(condition) = node.child_by_field_name("condition") {
        analysis.edits.push(Edit {
            position: condition.start_byte(),
            text: BUDGET_GUARD.to_owned(),
        });
    }
    if let Some(iterable) = node.child_by_field_name("right") {
        analysis.edits.push(Edit {
            position: iterable.start_byte(),
            text: format!("{ITER_GUARD}("),
        });
        analysis.edits.push(Edit {
            position: iterable.end_byte(),
            text: ")".to_owned(),
        });
    }
}

fn walk(
    node: Node<'_>,
    source: &str,
    filename: &str,
    depth: usize,
    analysis: &mut Analysis,
) -> Result<(), CompileError> {
    let line = u32::try_from(node.start_position().row.saturating_add(1)).unwrap_or(u32::MAX);
    if depth > MAX_DEPTH {
        return Err(CompileError::new(
            filename,
            line,
            "the guardrail program nests too deeply".to_owned(),
        ));
    }
    check_node(node, source, filename, line, analysis)?;
    let mut cursor = node.walk();
    let children: Vec<Node<'_>> = node.named_children(&mut cursor).collect();
    for child in children {
        walk(child, source, filename, depth.saturating_add(1), analysis)?;
    }
    Ok(())
}

fn record_function(
    node: Node<'_>,
    source: &str,
    filename: &str,
    line: u32,
    analysis: &mut Analysis,
) -> Result<(), CompileError> {
    let name = node.child_by_field_name("name").map_or("", |field| {
        source.get(field.byte_range()).unwrap_or_default()
    });
    check_name(name, filename, line, NameKind::Variable)?;
    if let Some(function) = module_function(node, name) {
        analysis.functions.push(function);
    }
    Ok(())
}

// Only a definition at module level is callable from outside; a nested one belongs to its enclosing body.
fn module_function(node: Node<'_>, name: &str) -> Option<Function> {
    if node.parent().is_none_or(|parent| parent.kind() != "module") {
        return None;
    }
    Some(Function {
        name: name.to_owned(),
        is_async: has_async(node),
    })
}

fn has_async(node: Node<'_>) -> bool {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .any(|child| child.kind() == "async")
}

// Applies the insertions from the end of the source backwards, so an earlier position stays valid.
pub fn apply(source: &str, edits: &[Edit], filename: &str) -> Result<String, CompileError> {
    let mut ordered: Vec<&Edit> = edits.iter().collect();
    ordered.sort_by_key(|edit| Reverse(edit.position));
    let mut rendered = source.to_owned();
    for edit in ordered {
        if edit.position > rendered.len() || !rendered.is_char_boundary(edit.position) {
            return Err(CompileError::new(
                filename,
                1,
                "the rewrite position is not a character boundary".to_owned(),
            ));
        }
        rendered.insert_str(edit.position, &edit.text);
    }
    Ok(rendered)
}
