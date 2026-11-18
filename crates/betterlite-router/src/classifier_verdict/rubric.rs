use serde::{Deserialize, Serialize};

use crate::complexity_scoring::tier::ComplexityTier;

use super::error::ClassifierVerdictError;

pub const CALIBRATION_EXAMPLES_HEADING: &str = "Calibration examples:";

const NON_REASONING_CRITERION: &str = "operational requests whose whole job is to pass information along or put it in a \
    requested shape: relaying or reformatting tool output, acknowledging a completed action, or extracting a stated \
    value. Use it only when no judgment about the content is asked for; the moment the request is to summarize, \
    compare, explain, debug, or decide, it belongs in a higher tier however short it is.";

const SIMPLE_CRITERION: &str = "greetings, chitchat, or factual lookups with a short known answer. Do not use this \
    tier for unsolved problems, proofs, deep theory, multi-step analysis, or non-trivial code, even if the request \
    is only one sentence.";

const MEDIUM_CRITERION: &str = "everyday requests that need some explanation, light reasoning, or minor code/technical content.";

const COMPLEX_CRITERION: &str =
    "non-trivial code, architecture, multi-step technical work, or specialized domain depth.";

const REASONING_CRITERION: &str = "open-ended analysis, proofs, famous hard problems, step-by-step reasoning, \
    tradeoffs, or anything where a correct answer requires careful thought rather than a quick lookup.";

const BUSINESS_RELAY_CRITERION: &str = "operational requests whose whole job is to pass information along \
    or put it in a requested shape: relaying or reformatting tool or system output, acknowledging a completed \
    action, or extracting a stated field. Use it only when no judgment about the content is asked for.";

const BUSINESS_SIMPLE_CRITERION: &str = "greetings, chitchat, or lookups of a fact, policy, price, or date with a \
    short known answer. Never for analysis, strategy, or non-trivial work, even if the request is only one \
    sentence.";

const BUSINESS_MEDIUM_CRITERION: &str = "everyday working requests: drafting, rewriting, summarizing, routine \
    explanations, light reasoning, or minor technical content, regardless of output length.";

const BUSINESS_COMPLEX_CRITERION: &str = "multi-step analysis or synthesis whose answer is determined by the \
    material at hand: diagnosing metrics from data, multi-source deliverables, non-trivial code, or specialized \
    domain depth.";

const BUSINESS_REASONING_CRITERION: &str = "committing to a decision under conflicting tradeoffs, genuine \
    optimization or proof, or anything where being right requires extended deliberation rather than applying a \
    known procedure.";

const CHAT_EXAMPLES: &str = r#"Calibration examples:
- "what's the capital of France?" -> {SIMPLE}
- three paragraphs of context ending in "what time does the building open on Saturdays?" -> {SIMPLE}, the ask is a lookup
- "Think step by step and reason carefully: what is 7 times 8?" -> {SIMPLE}, the framing does not change the task
- "in python, how do I check if a dict has a key?" -> {SIMPLE}, technical vocabulary but one obvious answer
- "write a regex for a US phone number" -> {MEDIUM}
- "explain REST vs gRPC and when to use each" -> {MEDIUM}
- "implement a distributed token bucket rate limiter on Redis, correct under concurrency" -> {COMPLEX}
- "prove the halting problem is undecidable" -> {COMPLEX} or {REASONING}, short but genuinely hard
- "should we use Postgres or Mongo given these constraints? commit to an answer" -> {REASONING}
- after a turn offering to work through a Raft safety argument, a bare "yes" -> {REASONING}, it inherits that work
- after a turn about the weather API, a bare "yes" -> {SIMPLE}, it inherits that work"#;

const AGENTIC_EXAMPLES: &str = r#"Calibration examples:
- "what's the capital of France?" -> {SIMPLE}
- three paragraphs of context ending in "what time does the building open on Saturdays?" -> {SIMPLE}, the ask is a lookup
- "Think step by step and reason carefully: what is 7 times 8?" -> {SIMPLE}, the framing does not change the task
- "in python, how do I check if a dict has a key?" -> {SIMPLE}, technical vocabulary but one obvious answer
- "write a regex for a US phone number" -> {MEDIUM}
- "explain REST vs gRPC and when to use each" -> {MEDIUM}
- "implement a distributed token bucket rate limiter on Redis, correct under concurrency" -> {COMPLEX}
- "why does our p99 latency triple when we double the replica count?" -> {COMPLEX}, casual and short, but the answer needs a real causal model
- "prove the halting problem is undecidable" -> {COMPLEX} or {REASONING}, short but genuinely hard
- "A farmer has 17 sheep. All but 9 die. How many are left?" -> {REASONING}, the arithmetic is trivial and the trap is not
- "should we use Postgres or Mongo given these constraints? commit to an answer" -> {REASONING}
- after a turn offering to work through a Raft safety argument, a bare "yes" -> {REASONING}, it inherits that work
- after a turn about the weather API, a bare "yes" -> {SIMPLE}, it inherits that work

Calibration on engineering tasks, which is where the boundary matters most. These are typical of agent and terminal work:
- "write /app/ode_solve.py, a small RK4 initial value problem solver, with the interface the tests import" -> {MEDIUM}
- "set up a Jupyter server with token auth on port 8888 and confirm it serves" -> {MEDIUM}
- "update this Fortran project's build to use gfortran instead of the legacy toolchain" -> {MEDIUM}
- "a secret was committed then removed by rewriting history; recover it and prove which commit introduced it" -> {MEDIUM}
- "complete the missing forward pass in this attention-based multiple instance learning model" -> {MEDIUM}
- "solve this 5x4 Huarong Dao sliding block puzzle in the fewest moves" -> {COMPLEX}, it needs a real search formulation
- "allocate rare-earth minerals across 1,000 variables under these constraints, optimally" -> {COMPLEX}
- "separability_matrix computes the wrong result for nested CompoundModels; find and fix the root cause" -> {COMPLEX}, the bug is in the semantics, not the syntax"#;

const BUSINESS_EXAMPLES: &str = r#"Calibration examples:
- "what's the capital of France?" -> {SIMPLE}
- three paragraphs of context ending in "what time does the building open on Saturdays?" -> {SIMPLE}, the ask is a lookup
- "Think step by step and reason carefully: what is 7 times 8?" -> {SIMPLE}, the framing does not change the task
- "in python, how do I check if a dict has a key?" -> {SIMPLE}, technical vocabulary but one obvious answer
- "write a regex for a US phone number" -> {MEDIUM}
- "explain REST vs gRPC and when to use each" -> {MEDIUM}
- "implement a distributed token bucket rate limiter on Redis, correct under concurrency" -> {COMPLEX}
- "prove the halting problem is undecidable" -> {COMPLEX} or {REASONING}, short but genuinely hard
- "should we use Postgres or Mongo given these constraints? commit to an answer" -> {REASONING}
- after a turn offering to work through a Raft safety argument, a bare "yes" -> {REASONING}, it inherits that work
- after a turn about the weather API, a bare "yes" -> {SIMPLE}, it inherits that work

Calibration on business and sales tasks, which is where the boundary matters most. Routine drafting, rewriting, and summarizing are everyday work, not analysis:
- "what's our refund policy?" -> {SIMPLE}
- a pasted email thread ending in "when does the Q3 promo end?" -> {SIMPLE}, the ask is a lookup
- "make this one-line reply to a customer sound friendlier" -> {SIMPLE}, one obvious transformation
- "draft a cold outreach email for a VP of Engineering at a fintech" -> {MEDIUM}
- "write an email to re-engage a prospect who went dark after the trial" -> {MEDIUM}, drafting that needs judgment is still routine work
- "summarize this discovery call transcript into next steps and owners" -> {MEDIUM}, long input but routine extraction
- "summarize what changed in this contract redline for a non-lawyer" -> {MEDIUM}
- "write a five-touch outreach sequence for this persona" -> {MEDIUM}, volume of output does not raise the tier
- "build a competitive battlecard against this vendor from these source docs" -> {COMPLEX}
- "here's our cohort table, diagnose why churn spiked" -> {COMPLEX}, hard analysis, but the data determines the answer
- "draft a counter-proposal for a multi-year enterprise renewal under these constraints" -> {COMPLEX}
- analysis that follows from supplied data is {COMPLEX} even when heavy with numbers; reserve {REASONING} for committing to a decision under conflicting tradeoffs or a genuine optimization
- "do we discount to close this quarter or hold price and risk slipping? commit to a recommendation" -> {REASONING}
- "design territories assigning our reps across these named accounts, optimally" -> {REASONING}"#;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClassificationRubric {
    #[default]
    Legacy,
    Agentic,
    Chat,
    Business,
}

impl ClassificationRubric {
    #[must_use]
    pub const fn examples(self) -> Option<&'static str> {
        match self {
            Self::Legacy => None,
            Self::Agentic => Some(AGENTIC_EXAMPLES),
            Self::Chat => Some(CHAT_EXAMPLES),
            Self::Business => Some(BUSINESS_EXAMPLES),
        }
    }

    #[must_use]
    pub const fn criterion(self, tier: ComplexityTier) -> &'static str {
        match (self, tier) {
            (Self::Business, ComplexityTier::NonReasoning) => BUSINESS_RELAY_CRITERION,
            (Self::Business, ComplexityTier::Simple) => BUSINESS_SIMPLE_CRITERION,
            (Self::Business, ComplexityTier::Medium) => BUSINESS_MEDIUM_CRITERION,
            (Self::Business, ComplexityTier::Complex) => BUSINESS_COMPLEX_CRITERION,
            (Self::Business, ComplexityTier::Reasoning) => BUSINESS_REASONING_CRITERION,
            (Self::Legacy | Self::Agentic | Self::Chat, ComplexityTier::NonReasoning) => {
                NON_REASONING_CRITERION
            }
            (Self::Legacy | Self::Agentic | Self::Chat, ComplexityTier::Simple) => SIMPLE_CRITERION,
            (Self::Legacy | Self::Agentic | Self::Chat, ComplexityTier::Medium) => MEDIUM_CRITERION,
            (Self::Legacy | Self::Agentic | Self::Chat, ComplexityTier::Complex) => {
                COMPLEX_CRITERION
            }
            (Self::Legacy | Self::Agentic | Self::Chat, ComplexityTier::Reasoning) => {
                REASONING_CRITERION
            }
        }
    }
}

pub fn calibration_examples_section(
    rubric: ClassificationRubric,
    labeled: &[(ComplexityTier, &str)],
) -> Result<String, ClassifierVerdictError> {
    let template = rubric
        .examples()
        .ok_or(ClassifierVerdictError::RubricExamples { rubric })?;
    let mut section = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        section.push_str(&rest[..open]);
        let tail = &rest[open + 1..];
        let Some(close) = tail.find('}') else {
            return Err(ClassifierVerdictError::RubricExamples { rubric });
        };
        let tier = ComplexityTier::built_in(&tail[..close])
            .ok_or(ClassifierVerdictError::RubricExamples { rubric })?;
        let label = labeled
            .iter()
            .find(|(candidate, _)| *candidate == tier)
            .map(|(_, label)| *label)
            .ok_or(ClassifierVerdictError::MissingLabel { tier: tier.name() })?;
        section.push_str(label);
        rest = &tail[close + 1..];
    }
    section.push_str(rest);
    Ok(section)
}
