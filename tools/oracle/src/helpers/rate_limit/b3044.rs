pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = serde_json::Value;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let object = input.as_object().ok_or_else(|| {
            Box::new(crate::helpers::HelperError {
                class: "InvalidInput".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: "usage resolution input must be an object".to_owned(),
            })
        })?;
        let usage = object.get("usage");
        let rate_type = object
            .get("rate_limit_type")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("total");
        let (total_tokens, input_tokens, output_tokens) = counts(usage);
        let selected = match rate_type {
            "input" => input_tokens.saturating_sub(cached(usage)),
            "output" => output_tokens,
            _ => total_tokens,
        };
        Ok(serde_json::json!({
            "rate_limit_type": if matches!(rate_type, "input" | "output" | "total") { rate_type } else { "total" },
            "total_tokens": selected,
            "input_tokens": input_tokens.saturating_sub(cached(usage)),
            "output_tokens": output_tokens,
        }))
    }
}

fn integer(source: Option<&serde_json::Value>) -> u64 {
    let Some(value) = source else {
        return 0;
    };
    value.as_u64().unwrap_or(0)
}

fn counts(usage: Option<&serde_json::Value>) -> (u64, u64, u64) {
    let total =
        integer(usage.and_then(|value| value.get("total_tokens"))).saturating_sub(cached(usage));
    let input = integer(
        usage
            .and_then(|value| value.get("prompt_tokens"))
            .or_else(|| {
                let value = usage?;
                value.get("input_tokens")
            }),
    );
    let output = integer(
        usage
            .and_then(|value| value.get("completion_tokens"))
            .or_else(|| {
                let value = usage?;
                value.get("output_tokens")
            }),
    );
    (total, input, output)
}

fn cached(value: Option<&serde_json::Value>) -> u64 {
    let nested = value
        .and_then(|usage| usage.get("prompt_tokens_details"))
        .and_then(serde_json::Value::as_object)
        .and_then(|details| details.get("cached_tokens"))
        .and_then(serde_json::Value::as_u64)
        .or_else(|| {
            value
                .and_then(|usage| usage.get("cache_read_input_tokens"))
                .and_then(serde_json::Value::as_u64)
        });
    nested.unwrap_or(0)
}
