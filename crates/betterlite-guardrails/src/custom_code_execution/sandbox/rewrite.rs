use std::cmp::Reverse;

use tree_sitter::{Language, Node, Parser, Tree};

use super::error::CompileError;

mod checks;
mod rules;

use rules::definitions::definition_rules;
use rules::{expression_rules, statement_rules};

const MAX_DEPTH: usize = 200;
const BUDGET_GUARD: &str = "_budget_ok_() and (";
const ITER_GUARD: &str = "_getiter_((";

// What the walker knows about its enclosing statement: return needs a function, break a loop, await an async one.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Scope {
    pub function_depth: usize,
    pub loop_depth: usize,
    pub is_async: bool,
}

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
    expression_rules(node, source, filename, line)?;
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
        _ => None,
    }
}

fn scope_after(node: Node<'_>, scope: Scope) -> Scope {
    if node.kind() == "function_definition" {
        let mut cursor = node.walk();
        return Scope {
            function_depth: scope.function_depth.saturating_add(1),
            loop_depth: 0,
            is_async: node
                .children(&mut cursor)
                .any(|child| child.kind() == "async"),
        };
    }
    if matches!(node.kind(), "for_statement" | "while_statement") {
        return Scope {
            loop_depth: scope.loop_depth.saturating_add(1),
            ..scope
        };
    }
    scope
}

// While tests and sync for iterables get their guards; an async for keeps its bare iterable, as visit_AsyncFor adds none.
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
            "the guardrail program nests too deeply",
        ));
    }
    check_node(node, source, filename, scope, analysis)?;
    let child_scope = scope_after(node, scope);
    let mut cursor = node.walk();
    let children: Vec<Node<'_>> = node.named_children(&mut cursor).collect();
    for child in children {
        walk(
            child,
            location,
            depth.saturating_add(1),
            child_scope,
            analysis,
        )?;
    }
    Ok(())
}

// Applies the insertions from the end of the source backwards, so an earlier position stays valid.
pub fn apply(source: &str, edits: &[Edit], filename: &str) -> Result<String, CompileError> {
    let mut ordered: Vec<&Edit> = edits.iter().collect();
    ordered.sort_by_key(|edit| Reverse(edit.position));
    let mut rendered = source.to_owned();
    for edit in ordered {
        if edit.position > rendered.len() || !rendered.is_char_boundary(edit.position) {
            return Err(CompileError::syntax(
                filename,
                1,
                "the rewrite position is not a character boundary",
            ));
        }
        rendered.insert_str(edit.position, &edit.text);
    }
    Ok(rendered)
}
