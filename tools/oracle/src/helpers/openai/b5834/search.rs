use std::collections::BTreeMap;

use betterlite_openai::openai_base_config::{Headers, Object as WireObject};
use betterlite_openai::perplexity_search::PerplexitySearch;
use betterlite_openai::perplexity_search::reply::decode_search_reply;
use betterlite_openai::search::provider::{RequestInput, SearchProvider as _};
use secrecy::SecretString;
use serde_json::{Value, json};

use super::{BaseCall, ReplyCall, SearchCall, SearchKeyCall};
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::HelperResult;

pub(super) fn values(
    headers: &[SearchKeyCall],
    urls: &[BaseCall],
    requests: &[SearchCall],
    replies: &[ReplyCall],
) -> HelperResult {
    let mut url_values = Vec::with_capacity(urls.len());
    for call in urls {
        let config = PerplexitySearch {
            environment: &call.env,
        };
        let url = config
            .complete_url(call.api_base.as_deref(), &WireObject::new(), None, None)
            .or_value_error()?;
        url_values.push(url);
    }
    let unset = BTreeMap::<String, String>::new();
    let config = PerplexitySearch {
        environment: &unset,
    };
    let caller_headers = Headers::new();
    let mut bodies = Vec::with_capacity(requests.len());
    for call in requests {
        let body = config
            .transform_search_request(&RequestInput {
                query: &call.query,
                optional_params: &call.optional_params,
                api_key: None,
                api_base: None,
                headers: &caller_headers,
            })
            .or_value_error()?;
        bodies.push(body);
    }
    Ok(json!({
        "headers": headers.iter().map(key_value).collect::<Vec<_>>(),
        "urls": url_values,
        "requests": bodies,
        "replies": replies.iter().map(reply_value).collect::<Vec<_>>(),
    }))
}

fn key_value(call: &SearchKeyCall) -> Value {
    let config = PerplexitySearch {
        environment: &call.env,
    };
    let key = call.api_key.clone().map(SecretString::from);
    config
        .validated_headers(call.headers.clone(), key.as_ref(), call.api_base.as_deref())
        .map_or_else(
            |_| json!({"raised": null}),
            |validated| json!({"ok": true, "headers": validated}),
        )
}

fn reply_value(call: &ReplyCall) -> Value {
    decode_search_reply(call.body.as_bytes()).map_or_else(
        |_| json!({"raised": null}),
        |reply| {
            let results: Vec<Value> = reply
                .results
                .iter()
                .map(|result| {
                    json!({
                        "title": result.title,
                        "url": result.url,
                        "snippet": result.snippet,
                        "date": result.date,
                        "last_updated": result.last_updated,
                    })
                })
                .collect();
            json!({"ok": true, "value": {"results": results, "object": reply.object.0}})
        },
    )
}
