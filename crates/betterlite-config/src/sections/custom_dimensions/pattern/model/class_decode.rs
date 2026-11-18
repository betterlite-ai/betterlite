use regex_syntax::ast;

use super::super::PatternProblem;
use super::{ClassItem, PyNode, category};

pub(super) fn node(bracketed: &ast::ClassBracketed) -> Result<PyNode, PatternProblem> {
    let mut items = Vec::new();
    decode_set(&bracketed.kind, &mut items)?;
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

fn decode_set(set: &ast::ClassSet, items: &mut Vec<ClassItem>) -> Result<(), PatternProblem> {
    match set {
        ast::ClassSet::Item(item) => decode_item(item, items),
        ast::ClassSet::BinaryOp(_) => Err(PatternProblem::Unsupported),
    }
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
