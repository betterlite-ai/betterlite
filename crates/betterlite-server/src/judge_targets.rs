pub mod call;
pub mod dispatcher;
pub mod error;
pub mod router;
pub mod sdk;

use std::collections::BTreeSet;

use betterlite_models::decisions::Json;
use betterlite_models::judge_primitives::target::{JudgeTarget, JudgeVia};
use indexmap::IndexMap;

use crate::judge_targets::call::JudgeCall;
use crate::judge_targets::dispatcher::JudgeDispatcher;
use crate::judge_targets::error::JudgeCallError;

pub fn judge_target(
    dispatcher: &JudgeDispatcher<'_>,
    model: &str,
    team_id: Option<&str>,
) -> JudgeTarget {
    let served = dispatcher.router.map_or_else(Vec::new, |router| {
        router.resolved_litellm_models(model, team_id)
    });
    if !served.is_empty() {
        let models = served
            .into_iter()
            .map(|served_model| {
                dispatcher
                    .cache
                    .qualified(&served_model, dispatcher.context)
                    .unwrap_or(served_model)
            })
            .collect::<BTreeSet<String>>();
        return JudgeTarget {
            via: JudgeVia::Router,
            models,
        };
    }
    dispatcher
        .cache
        .qualified(model, dispatcher.context)
        .map_or_else(
            || JudgeTarget {
                via: JudgeVia::Nothing,
                models: BTreeSet::new(),
            },
            |qualified| JudgeTarget {
                via: JudgeVia::Sdk,
                models: BTreeSet::from([qualified]),
            },
        )
}

pub async fn judge_acompletion(
    dispatcher: &JudgeDispatcher<'_>,
    judge_model: &str,
    messages: Vec<Json>,
    team_id: Option<&str>,
    params: IndexMap<String, Json>,
) -> Result<Json, JudgeCallError> {
    let via = judge_target(dispatcher, judge_model, team_id).via;
    let fallbacks = match via {
        JudgeVia::Router => Some(Vec::new()),
        JudgeVia::Sdk | JudgeVia::Nothing => None,
    };
    let call = JudgeCall {
        model: judge_model.to_owned(),
        messages,
        num_retries: 0,
        fallbacks,
        drop_params: true,
        params,
    };
    dispatch_judge_call(dispatcher, via, call, judge_model).await
}

async fn dispatch_judge_call(
    dispatcher: &JudgeDispatcher<'_>,
    via: JudgeVia,
    call: JudgeCall,
    judge_model: &str,
) -> Result<Json, JudgeCallError> {
    match via {
        JudgeVia::Router => match dispatcher.router {
            Some(router) => {
                router
                    .acompletion(call)
                    .await
                    .map_err(|source| JudgeCallError::Dispatch {
                        model: judge_model.to_owned(),
                        source,
                    })
            }
            None => Err(JudgeCallError::Nothing {
                model: judge_model.to_owned(),
            }),
        },
        JudgeVia::Sdk => {
            dispatcher
                .sdk
                .acompletion(call)
                .await
                .map_err(|source| JudgeCallError::Dispatch {
                    model: judge_model.to_owned(),
                    source,
                })
        }
        JudgeVia::Nothing => Err(JudgeCallError::Nothing {
            model: judge_model.to_owned(),
        }),
    }
}

mod tests;
