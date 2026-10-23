#![cfg(test)]

use crate::hook::task_from_branch;

#[test]
fn a_task_branch_names_its_task() {
    assert_eq!(task_from_branch("task/B004-openai-chat"), Some("B004"));
    assert_eq!(task_from_branch("task/B004"), Some("B004"));
}

#[test]
fn other_branches_name_no_task() {
    for branch in ["main", "task/B04-short", "task/B0045", "feature/B004-x"] {
        assert_eq!(task_from_branch(branch), None, "{branch}");
    }
}
