use betterlite_rate_limit::usage_resolution::{
    ResponseAPIUsage, ResponseObject, TokenRateLimitType, Usage, aggregate_only_total_tokens,
    get_rate_limit_type, get_total_tokens_from_usage, reconcile_io_usage,
    resolve_rerank_token_usage, response_usage,
};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) struct Input {
    rate_limit_type: Option<String>,
    usage: Option<Usage>,
    response_api_usage: Option<ResponseAPIUsage>,
    #[serde(default)]
    usage_kind: UsageKind,
    rerank_tokens: Option<TokenPair>,
    billed_units_total_tokens: Option<u64>,
    combined_usage_object: Option<Usage>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum UsageKind {
    #[default]
    Usage,
    ResponseApi,
    Mapping,
    Missing,
}

#[derive(Debug, Deserialize)]
struct TokenPair {
    input_tokens: u64,
    output_tokens: u64,
}

pub(crate) struct Op;

fn response(input: &Input) -> ResponseObject {
    match input.usage_kind {
        UsageKind::Usage => ResponseObject::Usage(input.usage.clone().unwrap_or_default()),
        UsageKind::ResponseApi => {
            ResponseObject::ResponseApi(input.response_api_usage.clone().unwrap_or_default())
        }
        UsageKind::Mapping => ResponseObject::Mapping(input.usage.clone().unwrap_or_default()),
        UsageKind::Missing => ResponseObject::WithUsage(None),
    }
}

fn resolved_counts(
    rerank: Option<(u64, u64, bool)>,
    usage: Option<betterlite_rate_limit::usage_resolution::ResponseUsage<'_>>,
    response: &ResponseObject,
    rate_limit_type: TokenRateLimitType,
) -> Result<(u64, u64, u64, bool), Box<crate::helpers::HelperError>> {
    if let Some((rerank_input, rerank_output, resolved)) = rerank {
        let total = match rate_limit_type {
            TokenRateLimitType::Input => rerank_input,
            TokenRateLimitType::Output => rerank_output,
            TokenRateLimitType::Total => {
                rerank_input.checked_add(rerank_output).ok_or_else(|| {
                    Box::new(crate::helpers::HelperError {
                        class: "InvalidInput".to_owned(),
                        status: None,
                        error_type: None,
                        param: None,
                        code: None,
                        message: "token count overflow".to_owned(),
                    })
                })?
            }
        };
        return Ok((total, rerank_input, rerank_output, resolved));
    }
    let total = get_total_tokens_from_usage(usage, rate_limit_type);
    let (input_tokens, output_tokens, resolved) = reconcile_io_usage(response_usage(response));
    Ok((total, input_tokens, output_tokens, resolved))
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let configured = TokenRateLimitType::from_config(input.rate_limit_type.as_deref());
        let rate_limit_name = get_rate_limit_type(configured);
        let rerank = resolve_rerank_token_usage(
            input
                .rerank_tokens
                .as_ref()
                .map(|tokens| (tokens.input_tokens, tokens.output_tokens)),
            input.billed_units_total_tokens,
        );
        let response = response(input);
        let usage = response_usage(&response);
        let (total_tokens, input_tokens, output_tokens, resolved) =
            resolved_counts(rerank, usage, &response, configured)?;
        let aggregate = aggregate_only_total_tokens(response_usage(&response));
        let recovered = input
            .combined_usage_object
            .as_ref()
            .map_or((0, 0, 0), |combined_usage| {
                betterlite_rate_limit::usage_resolution::recovered_partial_usage_tokens(
                    Some(combined_usage),
                    configured,
                )
            });
        Ok(serde_json::json!({
            "rate_limit_type": rate_limit_name,
            "total_tokens": total_tokens,
            "input_tokens": input_tokens,
            "output_tokens": output_tokens,
            "usage_resolved": resolved,
            "aggregate_only_total_tokens": aggregate,
            "recovered": [recovered.0, recovered.1, recovered.2],
        }))
    }
}
