use regex::Regex;

use crate::complexity_math;
use crate::complexity_scoring::config::Config;
use crate::complexity_scoring::decision::RoutingDecisionCause;
use crate::complexity_scoring::dimensions::{
    CODE_PRESENCE, CustomDimensionMatcher, DimensionScore, REASONING_MARKERS, SIMPLE_INDICATORS,
    TECHNICAL_TERMS, estimated_tokens, score_custom_dimensions, score_keyword_match,
    score_multi_step, score_question_complexity, score_token_count,
};
use crate::complexity_scoring::error::ScoringError;
use crate::complexity_scoring::keywords::{KeywordList, append_keywords, compile_patterns};
use crate::complexity_scoring::tier::{ComplexityTier, Tier};

// the system prompt is only carried for the classifier paths; the local scorer never reads it
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PromptInput<'input> {
    pub prompt: &'input str,
    pub system_prompt: Option<&'input str>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClassificationOutcome {
    pub tier: Tier,
    pub score: f64,
    pub signals: Vec<String>,
    pub cause: RoutingDecisionCause,
}

struct Evaluation {
    built_in: Vec<DimensionScore>,
    custom: Vec<(DimensionScore, f64)>,
    reasoning_matches: usize,
}

pub struct Scorer {
    config: Config,
    code_keywords: KeywordList,
    reasoning_keywords: KeywordList,
    technical_keywords: KeywordList,
    simple_keywords: KeywordList,
    multi_step_patterns: Vec<Regex>,
    custom_dimensions: Vec<CustomDimensionMatcher>,
}

impl Scorer {
    pub fn new(config: Config) -> Result<Self, ScoringError> {
        let technical = append_keywords(
            &config.technical_keywords,
            &config.custom_technical_keywords,
        );
        let mut custom_dimensions = Vec::with_capacity(config.custom_dimensions.len());
        for dimension in &config.custom_dimensions {
            custom_dimensions.push(CustomDimensionMatcher::compile(dimension)?);
        }
        Ok(Self {
            code_keywords: KeywordList::compile(&config.code_keywords)?,
            reasoning_keywords: KeywordList::compile(&config.reasoning_keywords)?,
            technical_keywords: KeywordList::compile(&technical)?,
            simple_keywords: KeywordList::compile(&config.simple_keywords)?,
            multi_step_patterns: compile_patterns(&config.multi_step_patterns)?,
            custom_dimensions,
            config,
        })
    }

    pub fn score_and_classify(
        &self,
        input: &PromptInput<'_>,
    ) -> Result<ClassificationOutcome, ScoringError> {
        let evaluation = self.evaluation(input.prompt);
        let signals = evaluation
            .built_in
            .iter()
            .chain(
                evaluation
                    .custom
                    .iter()
                    .map(|(dimension, _weight)| dimension),
            )
            .filter_map(|dimension| dimension.signal.clone())
            .collect();
        let score = complexity_math::weighted_score(
            &evaluation.built_in,
            &self.config.dimension_weights,
            &evaluation.custom,
        );
        if !score.is_finite() {
            return Err(ScoringError::NonFiniteScore);
        }
        let (tier, cause) = self.tier_and_cause(score, evaluation.reasoning_matches);
        Ok(ClassificationOutcome {
            tier,
            score,
            signals,
            cause,
        })
    }

    pub fn classify(
        &self,
        input: &PromptInput<'_>,
    ) -> Result<(Tier, f64, Vec<String>), ScoringError> {
        let outcome = self.score_and_classify(input)?;
        Ok((outcome.tier, outcome.score, outcome.signals))
    }

    fn evaluation(&self, prompt: &str) -> Evaluation {
        let user_text = prompt.to_lowercase();
        let (code, _) = score_keyword_match(&user_text, &self.code_keywords, &CODE_PRESENCE);
        let (reasoning, reasoning_matches) =
            score_keyword_match(&user_text, &self.reasoning_keywords, &REASONING_MARKERS);
        let (technical, _) =
            score_keyword_match(&user_text, &self.technical_keywords, &TECHNICAL_TERMS);
        let (simple, _) =
            score_keyword_match(&user_text, &self.simple_keywords, &SIMPLE_INDICATORS);
        let built_in = vec![
            score_token_count(estimated_tokens(prompt), self.config.token_thresholds),
            code,
            reasoning,
            technical,
            simple,
            score_multi_step(&user_text, &self.multi_step_patterns),
            score_question_complexity(prompt),
        ];
        let custom = score_custom_dimensions(prompt, &user_text, &self.custom_dimensions);
        Evaluation {
            built_in,
            custom,
            reasoning_matches,
        }
    }

    fn tier_and_cause(&self, score: f64, reasoning_matches: usize) -> (Tier, RoutingDecisionCause) {
        let floor = self.config.reasoning_override_min_score;
        if reasoning_matches >= 2 && score >= floor {
            return (
                Tier::BuiltIn(ComplexityTier::Reasoning),
                RoutingDecisionCause::ReasoningOverride,
            );
        }
        (
            Tier::BuiltIn(complexity_math::tier_for(
                score,
                self.config.tier_boundaries,
            )),
            RoutingDecisionCause::HeuristicScorer,
        )
    }
}
