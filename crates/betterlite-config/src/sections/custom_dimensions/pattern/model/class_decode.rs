use regex_syntax::ast;

use super::super::PatternProblem;
use super::{ClassItem, PyNode, push_unique};

pub(super) fn node(bracketed: &ast::ClassBracketed) -> Result<PyNode, PatternProblem> {
    let mut items = Vec::new();
    match &bracketed.kind {
        ast::ClassSet::Item(item) => decode_item(item, &mut items)?,
        // a node, so a refusal earlier in the sequence still reports first
        ast::ClassSet::BinaryOp(_) => return Ok(PyNode::Unsupported),
    }
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
        ast::ClassSetItem::Literal(literal) => push_unique(items, ClassItem::Literal(literal.c)),
        ast::ClassSetItem::Range(range) => {
            push_unique(items, ClassItem::Range(range.start.c, range.end.c));
        }
        ast::ClassSetItem::Perl(perl) => push_unique(
            items,
            ClassItem::Category {
                negated: perl.negated,
                kind: perl.kind.clone(),
            },
        ),
        ast::ClassSetItem::Union(union) => {
            for member in &union.items {
                decode_item(member, items)?;
            }
        }
        ast::ClassSetItem::Empty(_)
        | ast::ClassSetItem::Ascii(_)
        | ast::ClassSetItem::Unicode(_)
        | ast::ClassSetItem::Bracketed(_) => return Err(PatternProblem::Unsupported),
    }
    Ok(())
}
