use super::model::{PyNode, class_atom_steps};
use super::{
    MAX_CUSTOM_PATTERN_DEPTH, MAX_CUSTOM_PATTERN_REPEAT, MAX_CUSTOM_PATTERN_WORK, PatternProblem,
};

type Cost = (u64, u64);

pub(super) fn sequence_cost(nodes: &[PyNode], depth: u64) -> Result<Cost, PatternProblem> {
    if depth > MAX_CUSTOM_PATTERN_DEPTH {
        return Err(PatternProblem::Depth);
    }
    let mut costs: Vec<Cost> = Vec::with_capacity(nodes.len());
    for node in nodes {
        costs.push(node_cost(node, depth)?);
    }
    let mut paths = 1_u64;
    let mut steps = 1_u64;
    for (node_paths, node_steps) in costs {
        steps = steps.saturating_add(node_steps.saturating_mul(paths));
        paths = paths.saturating_mul(node_paths);
    }
    if steps > MAX_CUSTOM_PATTERN_WORK {
        return Err(PatternProblem::Work);
    }
    Ok((paths, steps))
}

fn node_cost(node: &PyNode, depth: u64) -> Result<Cost, PatternProblem> {
    match node {
        PyNode::Literal(_) | PyNode::NotLiteral(_) | PyNode::Any | PyNode::Assertion(_) => {
            Ok((1, 1))
        }
        PyNode::Class { .. } => Ok((1, class_atom_steps(node))),
        PyNode::Unsupported => Err(PatternProblem::Unsupported),
        PyNode::Subpattern(body) => sequence_cost(body, depth.saturating_add(1)),
        PyNode::Branch(branches) => branch_cost(branches, depth),
        PyNode::Repeat { min, max, body } => repeat_cost(*min, *max, body),
    }
}

fn branch_cost(branches: &[Vec<PyNode>], depth: u64) -> Result<Cost, PatternProblem> {
    let mut paths = 0_u64;
    let mut steps = u64::try_from(branches.len()).unwrap_or(u64::MAX);
    for branch in branches {
        let (branch_paths, branch_steps) = sequence_cost(branch, depth.saturating_add(1))?;
        paths = paths.saturating_add(branch_paths);
        steps = steps.saturating_add(branch_steps);
    }
    Ok((paths, steps))
}

fn repeat_cost(min: u64, max: u64, body: &[PyNode]) -> Result<Cost, PatternProblem> {
    if max > MAX_CUSTOM_PATTERN_REPEAT {
        return Err(PatternProblem::Repeat);
    }
    let [only] = body else {
        return Err(PatternProblem::Repeat);
    };
    let atom = matches!(
        only,
        PyNode::Literal(_) | PyNode::NotLiteral(_) | PyNode::Any | PyNode::Class { .. }
    );
    if !atom {
        return Err(PatternProblem::Repeat);
    }
    let choices = max.saturating_sub(min).saturating_add(1);
    let steps = 1_u64
        .saturating_add(max.saturating_mul(class_atom_steps(only)))
        .saturating_add(choices);
    Ok((choices, steps))
}
