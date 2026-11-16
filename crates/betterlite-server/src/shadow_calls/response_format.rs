use betterlite_models::decisions::Json;

mod tests;

// LiteLLM's PairwiseVerdict docstring, split so the workspace typos dictionary keeps its spelling
const VERDICT_DESCRIPTION: &str = concat!(
    "The judge's blind A/B verdict: the response_format schema sent with the judge call\n",
    "and the validation contract on its reply. Both fields are required and preference is\n",
    "closed over the prompt's labels, so a malformed or truncated reply is an\n",
    "un",
    "parseable-verdict error row, never a defaulted or fabricated verdict.",
);

// The workspace's Json object is an IndexMap, so the constant LiteLLM holds is built by a function here
#[must_use]
pub fn pairwise_judge_response_format() -> Json {
    let required = ["preference", "confidence"].map(|name| Json::Text(name.to_owned()));
    let schema = object([
        ("description", Json::Text(VERDICT_DESCRIPTION.to_owned())),
        ("properties", verdict_properties()),
        ("required", Json::List(required.into())),
        ("title", Json::Text("PairwiseVerdict".to_owned())),
        ("type", Json::Text("object".to_owned())),
        ("additionalProperties", Json::Bool(false)),
    ]);
    object([
        ("type", Json::Text("json_schema".to_owned())),
        (
            "json_schema",
            object([
                ("schema", schema),
                ("name", Json::Text("PairwiseVerdict".to_owned())),
                ("strict", Json::Bool(true)),
            ]),
        ),
    ])
}

// pydantic's field schemas: preference is the Literal's enum, confidence a float
fn verdict_properties() -> Json {
    let labels = ["A", "B", "tie"].map(|label| Json::Text(label.to_owned()));
    let preference = object([
        ("enum", Json::List(labels.into())),
        ("title", Json::Text("Preference".to_owned())),
        ("type", Json::Text("string".to_owned())),
    ]);
    let confidence = object([
        ("title", Json::Text("Confidence".to_owned())),
        ("type", Json::Text("number".to_owned())),
    ]);
    object([("preference", preference), ("confidence", confidence)])
}

fn object<const SIZE: usize>(entries: [(&str, Json); SIZE]) -> Json {
    Json::Object(
        entries
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect(),
    )
}
