#![cfg(test)]

use std::iter::repeat_with;

use betterlite_models::decisions::Json;

use super::super::graphql::{ingest_steps, mutation_query, query_variables, step_variables};
use super::super::prepare::prepare_log_data;
use super::super::step::Step;
use super::support::{FixedIds, at, kwargs};

const START: &str = "2024-05-01T10:00:00.120Z";
const END: &str = "2024-05-01T10:00:01.500Z";

const VARIABLES_0: &str = "$id_0: String!\n            $threadId_0: String\n            $rootRunId_0: String\n            $type_0: StepType\n            $startTime_0: DateTime\n            $endTime_0: DateTime\n            $error_0: String\n            $input_0: Json\n            $output_0: Json\n            $metadata_0: Json\n            $parentId_0: String\n            $name_0: String\n            $tags_0: [String!]\n            $generation_0: GenerationPayloadInput\n            $scores_0: [ScorePayloadInput!]\n            $attachments_0: [AttachmentPayloadInput!]\n            ";
const INGEST_0: &str = "\n        step0: ingestStep(\n            id: $id_0\n            threadId: $threadId_0\n            rootRunId: $rootRunId_0\n            startTime: $startTime_0\n            endTime: $endTime_0\n            type: $type_0\n            error: $error_0\n            input: $input_0\n            output: $output_0\n            metadata: $metadata_0\n            parentId: $parentId_0\n            name: $name_0\n            tags: $tags_0\n            generation: $generation_0\n            scores: $scores_0\n            attachments: $attachments_0\n        ) {\n            ok\n            message\n        }\n    ";

const CALLBACK: &str = r#"{
    "model": "gpt-4o",
    "standard_logging_object": {"model_parameters": {}, "response": {}}
}"#;

fn steps(count: usize) -> Vec<Step> {
    let ids = FixedIds::default();
    repeat_with(|| prepare_log_data(&kwargs(CALLBACK), at(START), at(END), &ids).unwrap())
        .take(count)
        .collect()
}

fn with_index(block: &str, index: usize) -> String {
    if index == 0 {
        return block.to_owned();
    }
    block
        .replace("_0", &format!("_{index}"))
        .replace("step0", &format!("step{index}"))
}

#[test]
fn query_variables_declare_every_field_once_per_step() {
    let two = steps(2);
    let expected = format!("{VARIABLES_0}{}", with_index(VARIABLES_0, 1));
    assert_eq!(query_variables(&two), expected);
}

#[test]
fn ingest_steps_name_every_variable_once_per_step() {
    let two = steps(2);
    let expected = format!("{INGEST_0}{}", with_index(INGEST_0, 1));
    assert_eq!(ingest_steps(&two), expected);
}

#[test]
fn mutation_query_wraps_the_declarations_and_the_ingest_calls() {
    let one = steps(1);
    let expected = format!(
        "\n        mutation AddStep({VARIABLES_0}) {{\n        {INGEST_0}\n        }}\n        "
    );
    assert_eq!(mutation_query(&one), expected);
}

#[test]
fn mutations_carry_one_block_per_step() {
    let two = steps(2);
    let query = mutation_query(&two);
    assert_eq!(query.matches("ingestStep(").count(), 2);
    assert_eq!(query.matches("$id_0: String!").count(), 1);
    assert_eq!(query.matches("$id_1: String!").count(), 1);
}

#[test]
fn variables_keep_only_non_null_fields_with_the_step_suffix() {
    let variables = step_variables(&steps(2));
    let keys: Vec<&str> = variables.keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        [
            "id_0",
            "name_0",
            "type_0",
            "startTime_0",
            "endTime_0",
            "metadata_0",
            "generation_0",
            "id_1",
            "name_1",
            "type_1",
            "startTime_1",
            "endTime_1",
            "metadata_1",
            "generation_1",
        ]
    );
    assert_eq!(variables["id_0"], Json::Text("generated-0".to_owned()));
    assert_eq!(variables["id_1"], Json::Text("generated-1".to_owned()));
    let Json::Object(generation) = &variables["generation_0"] else {
        panic!("the generation variable is an object");
    };
    assert_eq!(generation["promptId"], Json::Null);
}
