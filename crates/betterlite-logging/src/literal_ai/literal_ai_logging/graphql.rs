use std::fmt::Write as _;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::step::Step;

// _steps_query_variables_builder: one block of variable declarations per step, in the step's index order.
#[must_use]
pub fn query_variables(steps: &[Step]) -> String {
    let mut generated = String::new();
    for index in 0..steps.len() {
        let _ignored = write!(
            generated,
            "$id_{index}: String!\n            $threadId_{index}: String\n            $rootRunId_{index}: String\n            $type_{index}: StepType\n            $startTime_{index}: DateTime\n            $endTime_{index}: DateTime\n            $error_{index}: String\n            $input_{index}: Json\n            $output_{index}: Json\n            $metadata_{index}: Json\n            $parentId_{index}: String\n            $name_{index}: String\n            $tags_{index}: [String!]\n            $generation_{index}: GenerationPayloadInput\n            $scores_{index}: [ScorePayloadInput!]\n            $attachments_{index}: [AttachmentPayloadInput!]\n            "
        );
    }
    generated
}

// _steps_ingest_steps_builder: one ingestStep call per step, each argument reading the step's variable.
#[must_use]
pub fn ingest_steps(steps: &[Step]) -> String {
    let mut generated = String::new();
    for index in 0..steps.len() {
        let _ignored = write!(
            generated,
            "\n        step{index}: ingestStep(\n            id: $id_{index}\n            threadId: $threadId_{index}\n            rootRunId: $rootRunId_{index}\n            startTime: $startTime_{index}\n            endTime: $endTime_{index}\n            type: $type_{index}\n            error: $error_{index}\n            input: $input_{index}\n            output: $output_{index}\n            metadata: $metadata_{index}\n            parentId: $parentId_{index}\n            name: $name_{index}\n            tags: $tags_{index}\n            generation: $generation_{index}\n            scores: $scores_{index}\n            attachments: $attachments_{index}\n        ) {{\n            ok\n            message\n        }}\n    "
        );
    }
    generated
}

// _steps_query_builder: the AddStep mutation wrapping the declarations and the ingestStep calls.
#[must_use]
pub fn mutation_query(steps: &[Step]) -> String {
    format!(
        "\n        mutation AddStep({}) {{\n        {}\n        }}\n        ",
        query_variables(steps),
        ingest_steps(steps)
    )
}

// _steps_variables_builder.serialize_step: every non-None step field named after its key and step index.
#[must_use]
pub fn step_variables(steps: &[Step]) -> IndexMap<String, Json> {
    let mut variables = IndexMap::new();
    for (index, step) in steps.iter().enumerate() {
        for (key, value) in step.to_fields() {
            if value != Json::Null {
                variables.insert(format!("{key}_{index}"), value);
            }
        }
    }
    variables
}
