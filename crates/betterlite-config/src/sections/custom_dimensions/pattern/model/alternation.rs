use regex_syntax::ast;

use super::super::PatternProblem;
use super::{ClassItem, PyNode, prefix_eq, push_unique, sequence};

pub(super) fn nodes(alternation: &ast::Alternation) -> Result<Vec<PyNode>, PatternProblem> {
    let mut branches = Vec::with_capacity(alternation.asts.len());
    for branch in &alternation.asts {
        branches.push(sequence(branch)?);
    }
    let mut prefix = Vec::new();
    hoist(&mut branches, &mut prefix);
    let mut nodes = prefix;
    match merge(&branches) {
        Some(node) => nodes.push(node),
        None => nodes.push(PyNode::Branch(branches)),
    }
    Ok(nodes)
}

fn hoist(branches: &mut [Vec<PyNode>], prefix: &mut Vec<PyNode>) {
    loop {
        let Some(candidate) = branches.first().and_then(|branch| branch.first()).cloned() else {
            return;
        };
        if !branches.iter().all(|branch| {
            branch
                .first()
                .is_some_and(|node| prefix_eq(node, &candidate))
        }) {
            return;
        }
        prefix.push(candidate);
        for branch in branches.iter_mut() {
            branch.remove(0);
        }
    }
}

fn merge(branches: &[Vec<PyNode>]) -> Option<PyNode> {
    let mut items: Vec<ClassItem> = Vec::new();
    for branch in branches {
        let [node] = branch.as_slice() else {
            return None;
        };
        match node {
            PyNode::Literal(character) => push_unique(&mut items, ClassItem::Literal(*character)),
            PyNode::Class {
                negated: false,
                items: class_items,
            } => {
                for item in class_items {
                    push_unique(&mut items, item.clone());
                }
            }
            _ => return None,
        }
    }
    Some(PyNode::Class {
        negated: false,
        items,
    })
}
