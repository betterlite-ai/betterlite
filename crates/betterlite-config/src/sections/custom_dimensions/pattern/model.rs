use regex_syntax::ast::{self, Ast};

use super::{MAX_REPEAT, PatternProblem};

mod alternation;
mod class_decode;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum PyNode {
    Literal(char),
    NotLiteral(char),
    Any,
    Assertion,
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

pub(super) fn parse(pattern: &str) -> Result<Vec<PyNode>, PatternProblem> {
    let mut parser = regex_syntax::ast::parse::ParserBuilder::new()
        .nest_limit(1000)
        .octal(true)
        .empty_min_range(true)
        .build();
    let ast = parser
        .parse(pattern)
        .map_err(|_error| PatternProblem::Syntax)?;
    lower(&ast)
}

fn lower(ast: &Ast) -> Result<Vec<PyNode>, PatternProblem> {
    match ast {
        Ast::Empty(_) | Ast::Flags(_) => Ok(Vec::new()),
        Ast::Concat(concat) => {
            let mut nodes = Vec::new();
            for child in &concat.asts {
                nodes.extend(lower(child)?);
            }
            Ok(nodes)
        }
        Ast::Alternation(alternation) => alternation::nodes(alternation),
        Ast::Group(group) if matches!(&group.kind, ast::GroupKind::NonCapturing(flags) if flags.items.is_empty()) => {
            lower(&group.ast)
        }
        node => Ok(vec![single(node)?]),
    }
}

fn single(ast: &Ast) -> Result<PyNode, PatternProblem> {
    match ast {
        Ast::Literal(literal) => Ok(PyNode::Literal(literal.c)),
        Ast::Dot(_) => Ok(PyNode::Any),
        Ast::Assertion(assertion) => match assertion.kind {
            ast::AssertionKind::StartLine
            | ast::AssertionKind::EndLine
            | ast::AssertionKind::StartText
            | ast::AssertionKind::EndText
            | ast::AssertionKind::WordBoundary
            | ast::AssertionKind::NotWordBoundary => Ok(PyNode::Assertion),
            _ => Err(PatternProblem::Unsupported),
        },
        Ast::ClassPerl(perl) => Ok(PyNode::Class {
            negated: false,
            items: vec![ClassItem::Category {
                negated: perl.negated,
                kind: category(&perl.kind),
            }],
        }),
        Ast::ClassBracketed(bracketed) => class_decode::node(bracketed),
        Ast::Repetition(repetition) => repeat_node(repetition),
        Ast::Group(group) => Ok(PyNode::Subpattern(lower(&group.ast)?)),
        _ => Ok(PyNode::Unsupported),
    }
}

fn repeat_node(repetition: &ast::Repetition) -> Result<PyNode, PatternProblem> {
    match &*repetition.ast {
        Ast::Assertion(_) => return Err(PatternProblem::Syntax),
        Ast::Repetition(inner) => {
            let possessive = repetition.greedy
                && inner.greedy
                && matches!(&repetition.op.kind, ast::RepetitionKind::OneOrMore)
                && !matches!(&*inner.ast, Ast::Repetition(_));
            if possessive {
                // a node, so a refusal earlier in the sequence still reports first
                return Ok(PyNode::Unsupported);
            }
            return Err(PatternProblem::Syntax);
        }
        _ => {}
    }
    let Some((min, max)) = repetition_bounds(&repetition.op.kind) else {
        return Err(PatternProblem::Syntax);
    };
    let body = match &*repetition.ast {
        Ast::Group(group) if matches!(&group.kind, ast::GroupKind::NonCapturing(flags) if flags.items.is_empty()) => {
            lower(&group.ast)?
        }
        other => vec![single(other)?],
    };
    Ok(PyNode::Repeat { min, max, body })
}

fn repetition_bounds(kind: &ast::RepetitionKind) -> Option<(u64, u64)> {
    let bounds = match kind {
        ast::RepetitionKind::ZeroOrOne => (0, 1),
        ast::RepetitionKind::ZeroOrMore => (0, MAX_REPEAT),
        ast::RepetitionKind::OneOrMore => (1, MAX_REPEAT),
        ast::RepetitionKind::Range(ast::RepetitionRange::Exactly(count)) => {
            (explicit(*count)?, explicit(*count)?)
        }
        ast::RepetitionKind::Range(ast::RepetitionRange::AtLeast(min)) => {
            (explicit(*min)?, MAX_REPEAT)
        }
        ast::RepetitionKind::Range(ast::RepetitionRange::Bounded(min, max)) => {
            (explicit(*min)?, explicit(*max)?)
        }
    };
    Some(bounds)
}

pub(super) fn category(kind: &ast::ClassPerlKind) -> Category {
    match kind {
        ast::ClassPerlKind::Digit => Category::Digit,
        ast::ClassPerlKind::Space => Category::Space,
        ast::ClassPerlKind::Word => Category::Word,
    }
}

fn explicit(count: u32) -> Option<u64> {
    let value = u64::from(count);
    (value < MAX_REPEAT).then_some(value)
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
    match left {
        PyNode::Subpattern(_) | PyNode::Branch(_) | PyNode::Repeat { .. } | PyNode::Unsupported => {
            false
        }
        _ => left == right,
    }
}

fn push_unique(items: &mut Vec<ClassItem>, item: ClassItem) {
    if !items.contains(&item) {
        items.push(item);
    }
}
