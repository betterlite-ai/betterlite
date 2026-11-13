use serde::Serialize;

use super::config::ResponseFormat;
use super::error::ForecastError;
use super::verdict::{Boundary, Rule};

pub(super) const SYSTEM_PROMPT: &str = concat!(
    "You are a task-level probability forecaster for a model router. You receive the\n",
    "task's opening instruction and, when present, its latest user follow-up, plus\n",
    "the qualitative capability card below.\n",
    "\n",
    "Forecast one binary event:\n",
    "\n",
    "SUCCESS means that the efficient agent completes the whole task correctly on\n",
    "one fresh run under the actual harness, tools, and budget, as judged by the\n",
    "final verifier. FAILURE means any other outcome. The two outcomes are\n",
    "exhaustive.\n",
    "\n",
    "Use only evidence in the instruction and the capability card. Do not assume\n",
    "hidden repository state, unmentioned tools, validators, documentation, access,\n",
    "or future work habits. Do not invent empirical counts, success rates, or base\n",
    "rates. The capability card is qualitative evidence, not a measured prior.\n",
    "\n",
    "# Assessment procedure\n",
    "\n",
    "1. State the crux: the hardest material requirement for whole-task success.\n",
    "2. Select the one capability rule that best describes the crux. Use\n",
    "   primary_rule=none and capability_boundary=unmatched when no rule applies.\n",
    "   Rule ids are opaque labels. Do not infer a boundary from an id's spelling.\n",
    "3. Privately identify the strongest instruction-visible reasons for SUCCESS\n",
    "   and FAILURE, then imagine the most likely concrete failure.\n",
    "4. Privately consider material unknowns. Missing information should limit\n",
    "   extreme estimates, but it is not evidence that p_solve must equal 0.50.\n",
    "5. Estimate p_solve last. It is the probability of whole-task SUCCESS, not\n",
    "   confidence in this assessment, a route recommendation, or a cost judgment.\n",
    "\n",
    "Interpret probabilities as natural frequencies. If p_solve is 0.70 for 100\n",
    "comparable fresh runs, about 70 should succeed and 30 should fail. Use the full\n",
    "range when justified. Reserve 0.00 and 1.00 for outcomes that are logically\n",
    "impossible or certain under the visible contract. Supported does not mean 1.00,\n",
    "and unsupported does not mean 0.00. The downstream routing threshold is not\n",
    "part of this forecast.\n",
    "\n",
    "# Efficient-agent capability card\n",
    "\n",
    "The route verbs in this source card are inherited qualitative descriptions.\n",
    "They do not ask you to output a route and do not assign a fixed probability to\n",
    "any boundary.\n",
    "\n",
    "- SUP-1 [supported]: Route to the Efficient model when the task provides a complete output contract and a deterministic local validator that covers the material requirements.\n",
    "- SUP-2 [supported]: Route to the Efficient model when all required inputs are available, the target environment can be inspected, and correctness can be verified end-to-end without inaccessible external state.\n",
    "- SUP-3 [supported]: Route to the Efficient model when mathematical behavior, interfaces, shapes, data types, tolerances, and performance requirements are explicit and exercised by a representative harness.\n",
    "- SUP-4 [supported]: Route to the Efficient model when the required mechanism is identified, the relevant search space is bounded, and the success condition is executable. Do not infer this rule merely from the task's technical domain.\n",
    "- SUP-5 [supported]: Route to the Efficient model when reconstruction or behavioral reproduction is constrained by an executable reference, parser, format specification, or checker strong enough to distinguish correct from merely plausible output.\n",
    "- UNC-1 [uncertain]: Treat the route as uncertain when multiple reasonable interpretations of preprocessing, representation, indexing, naming, or output placement would produce different results and neither the instructions nor a validator resolve the choice.\n",
    "- UNC-2 [uncertain]: Treat the route as uncertain when success requires finding every relevant item across heterogeneous inputs or environment state, but the task does not define the search boundary or provide a completeness check.\n",
    "- LIM-1 [unsupported]: Prefer the Capable model when correctness depends primarily on extracting precise information from noisy visual, temporal, or rendered media and no machine-checkable extraction or replay mechanism is available.\n",
    "- LIM-2 [unsupported]: Prefer the Capable model when success depends on reproducing undocumented reference behavior, hidden intermediate state, or an unknown configuration, and small deviations fail despite satisfying the visible specification.\n",
    "\n",
    "# Output\n",
    "\n",
    "Return exactly one JSON object matching the response schema supplied with the\n",
    "request. Do not include markdown or commentary.\n",
    "\n",
    "p_solve must be between 0.00 and 1.00. p_fail is exactly 1.00 - p_solve and\n",
    "must not be emitted separately. Do not output recommended_route, confidence,\n",
    "abstain, counts, task totals, empirical rates, or any other field.",
);

const JSON_OBJECT_SUFFIX: &str = "\n\nReturn exactly one JSON object matching this JSON Schema:\n";

const RULE_CHOICES: [Rule; 10] = [
    Rule::Sup1,
    Rule::Sup2,
    Rule::Sup3,
    Rule::Sup4,
    Rule::Sup5,
    Rule::Unc1,
    Rule::Unc2,
    Rule::Lim1,
    Rule::Lim2,
    Rule::None,
];

const BOUNDARY_CHOICES: [Boundary; 4] = [
    Boundary::Supported,
    Boundary::Uncertain,
    Boundary::Unsupported,
    Boundary::Unmatched,
];

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ResponseFormatBody {
    JsonObject,
    JsonSchema { json_schema: Box<JsonSchemaWrapper> },
}

#[derive(Debug, Serialize)]
pub struct JsonSchemaWrapper {
    pub name: &'static str,
    pub strict: bool,
    pub schema: Schema,
}

// declaration order is the packaged literal's order; the json_object prompt re-sorts the schema through Value
#[derive(Debug, Serialize)]
pub struct Schema {
    #[serde(rename = "type")]
    pub kind: &'static str,
    #[serde(rename = "additionalProperties")]
    pub additional_properties: bool,
    pub required: [&'static str; 4],
    pub properties: Properties,
}

#[derive(Debug, Serialize)]
pub struct Properties {
    pub crux: CruxSchema,
    pub primary_rule: RuleSchema,
    pub capability_boundary: BoundarySchema,
    pub p_solve: SolveSchema,
}

#[derive(Debug, Serialize)]
pub struct CruxSchema {
    #[serde(rename = "type")]
    pub kind: &'static str,
    #[serde(rename = "minLength")]
    pub min_length: u8,
}

#[derive(Debug, Serialize)]
pub struct RuleSchema {
    #[serde(rename = "type")]
    pub kind: &'static str,
    #[serde(rename = "enum")]
    pub choices: [Rule; 10],
}

#[derive(Debug, Serialize)]
pub struct BoundarySchema {
    #[serde(rename = "type")]
    pub kind: &'static str,
    #[serde(rename = "enum")]
    pub choices: [Boundary; 4],
}

#[derive(Debug, Serialize)]
pub struct SolveSchema {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub minimum: f64,
    pub maximum: f64,
}

#[must_use]
pub fn response_format(mode: ResponseFormat) -> ResponseFormatBody {
    match mode {
        ResponseFormat::JsonObject => ResponseFormatBody::JsonObject,
        ResponseFormat::JsonSchema => ResponseFormatBody::JsonSchema {
            json_schema: Box::new(JsonSchemaWrapper {
                name: "CapabilityClassifierDecision",
                strict: true,
                schema: schema(),
            }),
        },
    }
}

pub(super) const fn schema() -> Schema {
    Schema {
        additional_properties: false,
        properties: Properties {
            capability_boundary: BoundarySchema {
                choices: BOUNDARY_CHOICES,
                kind: "string",
            },
            crux: CruxSchema {
                min_length: 1,
                kind: "string",
            },
            p_solve: SolveSchema {
                maximum: 1.0_f64,
                minimum: 0.0_f64,
                kind: "number",
            },
            primary_rule: RuleSchema {
                choices: RULE_CHOICES,
                kind: "string",
            },
        },
        required: ["crux", "primary_rule", "capability_boundary", "p_solve"],
        kind: "object",
    }
}

pub fn system_prompt(mode: ResponseFormat) -> Result<String, ForecastError> {
    match mode {
        ResponseFormat::JsonSchema => Ok(SYSTEM_PROMPT.to_owned()),
        ResponseFormat::JsonObject => {
            let mut prompt = SYSTEM_PROMPT.to_owned();
            prompt.push_str(JSON_OBJECT_SUFFIX);
            // serde_json::Map sorts its keys, so going through Value matches json.dumps(sort_keys=True)
            let sorted = serde_json::to_value(schema())
                .and_then(|value| serde_json::to_string_pretty(&value))
                .map_err(|source| ForecastError::Serialization { source })?;
            prompt.push_str(&sorted);
            Ok(prompt)
        }
    }
}
