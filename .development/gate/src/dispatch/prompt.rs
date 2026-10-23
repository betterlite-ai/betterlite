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
