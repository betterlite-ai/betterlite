use regex_syntax::ast::{self, Ast};

use super::{MAX_REPEAT, PatternProblem};

mod alternation;
mod class_decode;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum PyNode {
    Literal(char),
    NotLiteral(char),
    Any,
    Assertion(AssertionKind),
    Class {
        negated: bool,
        items: Vec<ClassItem>,
    },
    Unsupported,
    Subpattern(Vec<PyNode>),
    Branch(Vec<Vec<PyNode>>),
    Repeat {
        min: u64,
        max: u64,
        body: Vec<PyNode>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AssertionKind {
    StartLine,
    EndLine,
    StartText,
    EndText,
    WordBoundary,
    NotWordBoundary,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ClassItem {
    Literal(char),
    Range(char, char),
    Category { negated: bool, kind: Category },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Category {
    Digit,
    Space,
    Word,
}

pub(super) fn parse(canonical: &str) -> Result<Vec<PyNode>, PatternProblem> {
    let mut parser = regex_syntax::ast::parse::ParserBuilder::new()
        .nest_limit(1000)
        .build();
    let ast = parser
        .parse(canonical)
        .map_err(|_source| PatternProblem::Syntax)?;
    sequence(&ast)
}

fn sequence(ast: &Ast) -> Result<Vec<PyNode>, PatternProblem> {
    let mut nodes = Vec::new();
    append_sequence(&mut nodes, ast)?;
    Ok(nodes)
}

fn append_sequence(nodes: &mut Vec<PyNode>, ast: &Ast) -> Result<(), PatternProblem> {
    match ast {
        Ast::Empty(_) => Ok(()),
        Ast::Concat(concat) => {
            for child in &concat.asts {
                append_sequence(nodes, child)?;
            }
            Ok(())
        }
        Ast::Alternation(value) => {
            nodes.extend(alternation::nodes(value)?);
            Ok(())
        }
        Ast::Group(group) if is_plain_group(group) => append_sequence(nodes, &group.ast),
        _ => {
            nodes.push(lower_node(ast)?);
            Ok(())
        }
    }
}

fn lower_node(ast: &Ast) -> Result<PyNode, PatternProblem> {
    match ast {
        Ast::Literal(literal) => Ok(PyNode::Literal(literal.c)),
        Ast::Dot(_) => Ok(PyNode::Any),
        Ast::Assertion(assertion) => Ok(PyNode::Assertion(assertion_kind(&assertion.kind)?)),
        Ast::ClassPerl(perl) => Ok(PyNode::Class {
            negated: false,
            items: vec![ClassItem::Category {
                negated: perl.negated,
                kind: category(&perl.kind),
            }],
        }),
        Ast::ClassBracketed(bracketed) => class_decode::node(bracketed),
        Ast::ClassUnicode(_) | Ast::Flags(_) => Ok(PyNode::Unsupported),
        Ast::Repetition(repetition) => repeat_node(repetition),
        Ast::Group(group) => group_node(group),
        Ast::Alternation(_) | Ast::Concat(_) | Ast::Empty(_) => Ok(PyNode::Unsupported),
    }
}

fn repeat_node(repetition: &ast::Repetition) -> Result<PyNode, PatternProblem> {
    if matches!(&*repetition.ast, Ast::Repetition(_) | Ast::Assertion(_)) {
        return Err(PatternProblem::Syntax);
    }
    let (min, max) = repetition_bounds(&repetition.op.kind);
    let body = match &*repetition.ast {
        Ast::Group(group) if is_plain_group(group) => sequence(&group.ast)?,
        other => vec![lower_node(other)?],
    };
    Ok(PyNode::Repeat { min, max, body })
}

fn repetition_bounds(kind: &ast::RepetitionKind) -> (u64, u64) {
    match kind {
        ast::RepetitionKind::ZeroOrOne => (0, 1),
        ast::RepetitionKind::ZeroOrMore => (0, MAX_REPEAT),
        ast::RepetitionKind::OneOrMore => (1, MAX_REPEAT),
        ast::RepetitionKind::Range(ast::RepetitionRange::Exactly(count)) => {
            (u64::from(*count), u64::from(*count))
        }
        ast::RepetitionKind::Range(ast::RepetitionRange::AtLeast(min)) => {
            (u64::from(*min), MAX_REPEAT)
        }
        ast::RepetitionKind::Range(ast::RepetitionRange::Bounded(min, max)) => {
            (u64::from(*min), u64::from(*max))
        }
    }
}

fn group_node(group: &ast::Group) -> Result<PyNode, PatternProblem> {
    match &group.kind {
        ast::GroupKind::CaptureName { name, .. } => {
            let body = sequence(&group.ast)?;
            if name.name.starts_with('u') {
                Ok(PyNode::Unsupported)
            } else {
                Ok(PyNode::Subpattern(body))
            }
        }
        ast::GroupKind::CaptureIndex(_) | ast::GroupKind::NonCapturing(_) => {
            Ok(PyNode::Subpattern(sequence(&group.ast)?))
        }
    }
}

fn is_plain_group(group: &ast::Group) -> bool {
    matches!(&group.kind, ast::GroupKind::NonCapturing(flags) if flags.items.is_empty())
}

fn assertion_kind(kind: &ast::AssertionKind) -> Result<AssertionKind, PatternProblem> {
    match kind {
        ast::AssertionKind::StartLine => Ok(AssertionKind::StartLine),
        ast::AssertionKind::EndLine => Ok(AssertionKind::EndLine),
        ast::AssertionKind::StartText => Ok(AssertionKind::StartText),
        ast::AssertionKind::EndText => Ok(AssertionKind::EndText),
        ast::AssertionKind::WordBoundary => Ok(AssertionKind::WordBoundary),
        ast::AssertionKind::NotWordBoundary => Ok(AssertionKind::NotWordBoundary),
        _ => Err(PatternProblem::Unsupported),
    }
}

fn category(kind: &ast::ClassPerlKind) -> Category {
    match kind {
        ast::ClassPerlKind::Digit => Category::Digit,
        ast::ClassPerlKind::Space => Category::Space,
        ast::ClassPerlKind::Word => Category::Word,
    }
}

pub(super) fn is_atom(node: &PyNode) -> bool {
    matches!(
        node,
        PyNode::Literal(_) | PyNode::NotLiteral(_) | PyNode::Any | PyNode::Class { .. }
    )
}

pub(super) fn class_atom_steps(node: &PyNode) -> u64 {
    match node {
        PyNode::Class { negated, items } => {
            let count = u64::try_from(items.len()).unwrap_or(u64::MAX);
            count.saturating_add(1).saturating_add(u64::from(*negated))
        }
        _ => 1,
    }
}

fn prefix_eq(left: &PyNode, right: &PyNode) -> bool {
    match (left, right) {
        (PyNode::Literal(one), PyNode::Literal(other)) => one == other,
        (PyNode::NotLiteral(one), PyNode::NotLiteral(other)) => one == other,
        (PyNode::Any, PyNode::Any) => true,
        (PyNode::Assertion(one), PyNode::Assertion(other)) => one == other,
        (
            PyNode::Class {
                negated: one_negated,
                items: one_items,
            },
            PyNode::Class {
                negated: other_negated,
                items: other_items,
            },
        ) => one_negated == other_negated && one_items == other_items,
        _ => false,
    }
}

fn push_unique(items: &mut Vec<ClassItem>, item: ClassItem) {
    if !items.contains(&item) {
        items.push(item);
    }
}
