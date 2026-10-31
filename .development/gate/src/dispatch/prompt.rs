mod tests;

use crate::dispatch::EarlierReview;
use crate::spec::task::Task;

pub(crate) struct Brief<'task> {
    pub(crate) task: &'task Task,
    pub(crate) file: &'task str,
    pub(crate) branch: &'task str,
    pub(crate) base: &'task str,
}

pub(crate) fn worker(brief: &Brief<'_>, feedback: &[String]) -> String {
    let mut prompt = format!(
        "You are the betterlite worker for task {id}: {title}.\nRead .development/roles/worker.md first and follow it. The rules are in .development/rules.yaml and the task is .development/{file}.\nYou are on branch {branch} in your own worktree. Stay on it and commit your work there.\nYour final answer is the report JSON described in .development/schema/report.schema.json.\n",
        id = brief.task.id,
        title = brief.task.title,
        file = brief.file,
        branch = brief.branch,
    );
    if !feedback.is_empty() {
        let items: Vec<String> = feedback.iter().map(|item| format!("- {item}")).collect();
        prompt.push_str("\nThe previous attempt did not pass. Fix these first:\n");
        prompt.push_str(&items.join("\n"));
        prompt.push('\n');
    }
    prompt
}

pub(crate) fn fix_turn(feedback: &[String]) -> String {
    let items: Vec<String> = feedback.iter().map(|item| format!("- {item}")).collect();
    format!(
        "Your last run did not pass. Fix these first:\n{items}\nYour final answer is the report JSON again.\n",
        items = items.join("\n"),
    )
}

pub(crate) fn reviewer(brief: &Brief<'_>, earlier: Option<&EarlierReview>) -> String {
    let rounds = earlier.map_or_else(String::new, |review| {
        if review.findings.is_empty() {
            return format!(
                "\nAn earlier round approved this branch at {commit}. Report findings only for lines changed since then (git diff {commit}...HEAD); the rest is settled.\n",
                commit = review.commit,
            );
        }
        let items: Vec<String> = review
            .findings
            .iter()
            .map(|item| format!("- {item}"))
            .collect();
        format!(
            "\nAn earlier round reviewed this branch at {commit} and asked for these changes:\n{items}\nCheck that each one is fixed. Report a finding only for one that is not fixed, or for a line changed since that round (git diff {commit}...HEAD). Code the earlier round already read is settled.\n",
            commit = review.commit,
            items = items.join("\n"),
        )
    });
    format!(
        "You are the betterlite reviewer for task {id}: {title}.\nRead .development/roles/reviewer.md first and follow it. The rules are in .development/rules.yaml and the task is .development/{file}.\nReview branch {branch} against its base {base} with git diff {base}...HEAD. Change nothing.\nYour final answer is the review JSON described in .development/schema/review.schema.json.\n{rounds}",
        id = brief.task.id,
        title = brief.task.title,
        file = brief.file,
        branch = brief.branch,
        base = brief.base,
    )
}
