use regex_syntax::ast;

use super::super::PatternProblem;
use super::{ClassItem, PyNode, category};

pub(super) fn node(bracketed: &ast::ClassBracketed) -> Result<PyNode, PatternProblem> {
    let mut items = Vec::new();
    match &bracketed.kind {
        ast::ClassSet::Item(item) => decode_item(item, &mut items)?,
        // a node, so a refusal earlier in the sequence still reports first
        ast::ClassSet::BinaryOp(_) => return Ok(PyNode::Unsupported),
    }
    dedupe(&mut items);
    if let [ClassItem::Literal(character)] = items.as_slice() {
        if bracketed.negated {
            return Ok(PyNode::NotLiteral(*character));
        }
        return Ok(PyNode::Literal(*character));
    }
    Ok(PyNode::Class {
        negated: bracketed.negated,
        items,
    })
}

fn decode_item(item: &ast::ClassSetItem, items: &mut Vec<ClassItem>) -> Result<(), PatternProblem> {
    match item {
        ast::ClassSetItem::Literal(literal) => {
            items.push(ClassItem::Literal(literal.c));
            Ok(())
        }
        ast::ClassSetItem::Range(range) => {
            items.push(ClassItem::Range(range.start.c, range.end.c));
            Ok(())
        }
        ast::ClassSetItem::Perl(perl) => {
            items.push(ClassItem::Category {
                negated: perl.negated,
                kind: category(&perl.kind),
            });
            Ok(())
        }
        ast::ClassSetItem::Union(union) => {
            for item in &union.items {
                decode_item(item, items)?;
            }
            Ok(())
        }
        _ => Err(PatternProblem::Unsupported),
    }
}

fn dedupe(items: &mut Vec<ClassItem>) {
    let mut unique: Vec<ClassItem> = Vec::with_capacity(items.len());
    for item in items.drain(..) {
        if !unique.contains(&item) {
            unique.push(item);
        }
    }
    *items = unique;
}
