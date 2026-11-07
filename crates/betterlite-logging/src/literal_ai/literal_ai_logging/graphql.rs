use std::fmt::Write as _;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::step::Step;

const STEP_VARIABLES: [(&str, &str); 16] = [
    ("id", "String!"),
    ("threadId", "String"),
    ("rootRunId", "String"),
    ("type", "StepType"),
    ("startTime", "DateTime"),
    ("endTime", "DateTime"),
    ("error", "String"),
    ("input", "Json"),
    ("output", "Json"),
    ("metadata", "Json"),
    ("parentId", "String"),
    ("name", "String"),
    ("tags", "[String!]"),
    ("generation", "GenerationPayloadInput"),
    ("scores", "[ScorePayloadInput!]"),
    ("attachments", "[AttachmentPayloadInput!]"),
];

// _steps_query_variables_builder: one block of variable declarations per step, in the step's index order.
#[must_use]
pub fn query_variables(steps: &[Step]) -> String {
    let mut generated = String::new();
    for index in 0..steps.len() {
        for (name, kind) in STEP_VARIABLES {
            let _ignored = write!(generated, "${name}_{index}: {kind}\n            ");
        }
    }
    generated
}

// _steps_ingest_steps_builder: one ingestStep call per step, each argument reading the step's variable.
#[must_use]
pub fn ingest_steps(steps: &[Step]) -> String {
    steps
        .iter()
        .enumerate()
        .fold(String::new(), |mut generated, (index, _)| {
            let _ignored = write!(generated, "\n        step{index}: ingestStep(\n            id: $id_{index}\n            threadId: $threadId_{index}\n            rootRunId: $rootRunId_{index}\n            startTime: $startTime_{index}\n            endTime: $endTime_{index}\n            type: $type_{index}\n            error: $error_{index}\n            input: $input_{index}\n            output: $output_{index}\n            metadata: $metadata_{index}\n            parentId: $parentId_{index}\n            name: $name_{index}\n            tags: $tags_{index}\n            generation: $generation_{index}\n            scores: $scores_{index}\n            attachments: $attachments_{index}\n        ) {{\n            ok\n            message\n        }}\n    ");
            generated
        })
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
