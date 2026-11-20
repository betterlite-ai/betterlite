use crate::model_usage::catalog::{
    MODEL_INSIGHT_TASKS, ModelInsightTask, load_model_insight_tasks,
};

#[test]
fn the_catalog_holds_the_twenty_pinned_tasks_in_file_order() {
    let listed: Vec<(&str, &str, &str)> = MODEL_INSIGHT_TASKS
        .iter()
        .map(|task| (task.task_type, task.label, task.category))
        .collect();
    assert_eq!(
        listed,
        [
            ("classification", "Classification", "General"),
            ("content_writing", "Content Writing", "General"),
            ("roleplay_fiction", "Roleplay & Fiction", "General"),
            ("conversation", "Conversation", "General"),
            ("research_reports", "Research & Reports", "General"),
            ("qa_knowledge", "Q&A & Knowledge", "General"),
            ("customer_support", "Customer Support", "General"),
            ("summarization", "Summarization", "General"),
            ("translation", "Translation", "General"),
            ("workflow_execution", "Workflow Execution", "Agent"),
            ("multi_step_planning", "Multi-step Planning", "Agent"),
            ("tool_dispatch", "Tool Dispatch", "Agent"),
            ("code_generation", "Code Generation", "Code"),
            ("debugging", "Debugging", "Code"),
            ("code_review", "Code Review", "Code"),
            ("frontend_ui", "Frontend & UI", "Code"),
            ("file_io", "File I/O", "Code"),
            ("shell_execution", "Shell Execution", "Code"),
            ("data_extraction", "Data Extraction", "Data"),
            ("data_transformation", "Data Transformation", "Data"),
        ]
    );
}

#[test]
fn the_loader_keys_every_task_by_its_type_in_catalog_order() {
    let tasks = load_model_insight_tasks();
    assert_eq!(tasks.len(), MODEL_INSIGHT_TASKS.len());
    let listed: Vec<&str> = tasks.keys().copied().collect();
    assert_eq!(
        listed,
        MODEL_INSIGHT_TASKS
            .iter()
            .map(|task| task.task_type)
            .collect::<Vec<_>>()
    );
    let review = tasks.get("code_review").expect("a known task");
    assert_eq!(
        *review,
        ModelInsightTask {
            task_type: "code_review",
            label: "Code Review",
            category: "Code",
        }
    );
    assert_eq!(tasks.get("code-review"), None);
}
