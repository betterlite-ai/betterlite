use indexmap::IndexMap;
use serde::Serialize;

use betterlite_models::decisions::systemone;
use betterlite_models::decisions::validators::Bounded;
use betterlite_models::decisions::{DecisionsJson, Extra};

#[derive(Serialize)]
pub struct NoulQuestionWire {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<DecisionsJson>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub criteria: Option<systemone::NoulCriteria>,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Serialize)]
pub struct ChoiceQuestionWire {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<DecisionsJson>,
    pub criteria: Bounded<IndexMap<String, Option<DecisionsJson>>, 1, 255>,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Serialize)]
pub struct ScoreQuestionWire {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<DecisionsJson>,
    pub criteria: Bounded<Vec<DecisionsJson>, 1, 10>,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum QuestionWire {
    Noul(NoulQuestionWire),
    Choice(ChoiceQuestionWire),
    Score(ScoreQuestionWire),
}

impl From<&systemone::DecisionQuestion> for QuestionWire {
    fn from(question: &systemone::DecisionQuestion) -> Self {
        match question {
            systemone::DecisionQuestion::Noul(checked) => {
                let noul = checked.as_ref();
                Self::Noul(NoulQuestionWire {
                    instructions: noul.instructions.clone(),
                    criteria: noul.criteria.clone(),
                    extra: noul.extra.clone(),
                })
            }
            systemone::DecisionQuestion::Choice(choice) => Self::Choice(ChoiceQuestionWire {
                instructions: choice.instructions.clone(),
                criteria: choice.criteria.clone(),
                extra: choice.extra.clone(),
            }),
            systemone::DecisionQuestion::Score(score) => Self::Score(ScoreQuestionWire {
                instructions: score.instructions.clone(),
                criteria: score.criteria.clone(),
                extra: score.extra.clone(),
            }),
        }
    }
}

#[derive(Serialize)]
pub struct RequestWire {
    pub model: String,
    pub state: DecisionsJson,
    pub questions: IndexMap<systemone::QuestionName, QuestionWire>,
    #[serde(flatten)]
    pub extra: Extra,
}

fn question_wires(
    questions: &systemone::DecisionQuestionMap,
) -> IndexMap<systemone::QuestionName, QuestionWire> {
    questions
        .as_ref()
        .iter()
        .map(|(name, question)| (name.clone(), QuestionWire::from(question)))
        .collect()
}

impl From<&systemone::DecisionsRequest> for RequestWire {
    fn from(request: &systemone::DecisionsRequest) -> Self {
        Self {
            model: request.model.clone(),
            state: request.body.state.clone(),
            questions: question_wires(&request.body.questions),
            extra: request.body.extra.clone(),
        }
    }
}
