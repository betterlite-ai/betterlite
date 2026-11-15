use betterlite_models::decisions::Json;

mod tests;

const VERDICT_DESCRIPTION: &str = r#"The judge's blind A/B verdict: the response_format schema sent with the judge call
and the validation contract on its reply. Both fields are required and preference is
closed over the prompt's labels, so a malformed or truncated reply is an
unparseable-verdict error row, never a defaulted or fabricated verdict."#;

// The workspace's Json object is an IndexMap, so the constant LiteLLM holds is built by a function here
#[must_use]
pub fn pairwise_judge_response_format() -> Json {
    object([
        ("type", Json::Text("json_schema".to_owned())),
        (
            "json_schema",
            object([
                ("schema", verdict_schema()),
                ("name", Json::Text("PairwiseVerdict".to_owned())),
                ("strict", Json::Bool(true)),
            ]),
        ),
    ])
}

fn verdict_schema() -> Json {
    object([
        ("description", Json::Text(VERDICT_DESCRIPTION.to_owned())),
        (
            "properties",
            object([
                ("preference", preference_schema()),
                ("confidence", confidence_schema()),
            ]),
        ),
        (
            "required",
            Json::List(vec![
                Json::Text("preference".to_owned()),
                Json::Text("confidence".to_owned()),
            ]),
        ),
        ("title", Json::Text("PairwiseVerdict".to_owned())),
        ("type", Json::Text("object".to_owned())),
        ("additionalProperties", Json::Bool(false)),
    ])
}

fn preference_schema() -> Json {
    object([
        (
            "enum",
            Json::List(vec![
                Json::Text("A".to_owned()),
                Json::Text("B".to_owned()),
                Json::Text("tie".to_owned()),
            ]),
        ),
        ("title", Json::Text("Preference".to_owned())),
        ("type", Json::Text("string".to_owned())),
    ])
}

fn confidence_schema() -> Json {
    object([
        ("title", Json::Text("Confidence".to_owned())),
        ("type", Json::Text("number".to_owned())),
    ])
}

fn object<const SIZE: usize>(entries: [(&str, Json); SIZE]) -> Json {
    Json::Object(
        entries
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect(),
    )
}
