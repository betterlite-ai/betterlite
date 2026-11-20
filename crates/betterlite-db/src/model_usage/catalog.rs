use indexmap::IndexMap;
use serde::Serialize;

// ModelInsightTask of litellm/types/model_insights.py, the record the task-summary route reads back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ModelInsightTask {
    pub task_type: &'static str,
    pub label: &'static str,
    pub category: &'static str,
}

// MODEL_INSIGHTS_TASKS of litellm/proxy/model_insights_tasks.json at the pin, in the file's order so a listing keeps the catalog's arrangement.
pub const MODEL_INSIGHT_TASKS: [ModelInsightTask; 20] = [
    ModelInsightTask {
        task_type: "classification",
        label: "Classification",
        category: "General",
    },
    ModelInsightTask {
        task_type: "content_writing",
        label: "Content Writing",
        category: "General",
    },
    ModelInsightTask {
        task_type: "roleplay_fiction",
        label: "Roleplay & Fiction",
        category: "General",
    },
    ModelInsightTask {
        task_type: "conversation",
        label: "Conversation",
        category: "General",
    },
    ModelInsightTask {
        task_type: "research_reports",
        label: "Research & Reports",
        category: "General",
    },
    ModelInsightTask {
        task_type: "qa_knowledge",
        label: "Q&A & Knowledge",
        category: "General",
    },
    ModelInsightTask {
        task_type: "customer_support",
        label: "Customer Support",
        category: "General",
    },
    ModelInsightTask {
        task_type: "summarization",
        label: "Summarization",
        category: "General",
    },
    ModelInsightTask {
        task_type: "translation",
        label: "Translation",
        category: "General",
    },
    ModelInsightTask {
        task_type: "workflow_execution",
        label: "Workflow Execution",
        category: "Agent",
    },
    ModelInsightTask {
        task_type: "multi_step_planning",
        label: "Multi-step Planning",
        category: "Agent",
    },
    ModelInsightTask {
        task_type: "tool_dispatch",
        label: "Tool Dispatch",
        category: "Agent",
    },
    ModelInsightTask {
        task_type: "code_generation",
        label: "Code Generation",
        category: "Code",
    },
    ModelInsightTask {
        task_type: "debugging",
        label: "Debugging",
        category: "Code",
    },
    ModelInsightTask {
        task_type: "code_review",
        label: "Code Review",
        category: "Code",
    },
    ModelInsightTask {
        task_type: "frontend_ui",
        label: "Frontend & UI",
        category: "Code",
    },
    ModelInsightTask {
        task_type: "file_io",
        label: "File I/O",
        category: "Code",
    },
    ModelInsightTask {
        task_type: "shell_execution",
        label: "Shell Execution",
        category: "Code",
    },
    ModelInsightTask {
        task_type: "data_extraction",
        label: "Data Extraction",
        category: "Data",
    },
    ModelInsightTask {
        task_type: "data_transformation",
        label: "Data Transformation",
        category: "Data",
    },
];

// load_model_insight_tasks: the catalog keyed by task type, rebuilt on each call because the repository keeps no global state.
#[must_use]
pub fn load_model_insight_tasks() -> IndexMap<&'static str, ModelInsightTask> {
    let mut tasks = IndexMap::with_capacity(MODEL_INSIGHT_TASKS.len());
    for task in MODEL_INSIGHT_TASKS {
        tasks.insert(task.task_type, task);
    }
    tasks
}
