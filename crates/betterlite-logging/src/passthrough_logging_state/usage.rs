use std::str;

use http::HeaderMap;

pub const UPSTREAM_RESPONSE_COST_HEADER: &str = "x-litellm-response-cost";
pub const UPSTREAM_TOTAL_TOKENS_HEADER: &str = "x-litellm-total-tokens";
// model_call_details key that marks an upstream which speaks the reported-usage contract
pub const UPSTREAM_REPORTED_USAGE_KEY: &str = "_litellm_upstream_reported_usage";

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UpstreamReportedUsage {
    // ast-grep-ignore: no-float-money
    pub response_cost: Option<f64>,
    pub total_tokens: Option<i64>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PassThroughCallState {
    pub upstream_reported_usage: Option<UpstreamReportedUsage>,
    // ast-grep-ignore: no-float-money
    pub response_cost: Option<f64>,
    pub combined_usage_total_tokens: Option<i64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HeaderEncoding {
    Ascii,
    Utf8,
    Latin1,
}

// httpx picks one encoding for the whole header map: ascii, then utf-8, then iso-8859-1
fn header_encoding(headers: &HeaderMap) -> HeaderEncoding {
    let mut ascii = true;
    let mut utf8 = true;
    for (_, value) in headers {
        let bytes = value.as_bytes();
        ascii &= bytes.is_ascii();
        utf8 &= str::from_utf8(bytes).is_ok();
    }
    if ascii {
        HeaderEncoding::Ascii
    } else if utf8 {
        HeaderEncoding::Utf8
    } else {
        HeaderEncoding::Latin1
    }
}

fn decode_value(value: &[u8], encoding: HeaderEncoding) -> String {
    match encoding {
        HeaderEncoding::Ascii | HeaderEncoding::Utf8 => {
            str::from_utf8(value).map_or_else(|_| latin1(value), str::to_owned)
        }
        HeaderEncoding::Latin1 => latin1(value),
    }
}

fn latin1(value: &[u8]) -> String {
    // ISO-8859-1 maps every byte to the code point with the same value
    value.iter().map(|byte| char::from(*byte)).collect()
}

// httpx's Headers.get joins repeated values with ", " in the order they were sent
fn joined_header(headers: &HeaderMap, name: &str, encoding: HeaderEncoding) -> Option<String> {
    let mut values = headers.get_all(name).iter();
    let first = values.next()?;
    let mut joined = decode_value(first.as_bytes(), encoding);
    for value in values {
        joined.push_str(", ");
        joined.push_str(&decode_value(value.as_bytes(), encoding));
    }
    Some(joined)
}

#[must_use]
pub fn parse_upstream_reported_usage(headers: &HeaderMap) -> Option<UpstreamReportedUsage> {
    let encoding = header_encoding(headers);
    let raw_response_cost = joined_header(headers, UPSTREAM_RESPONSE_COST_HEADER, encoding);
    let raw_total_tokens = joined_header(headers, UPSTREAM_TOTAL_TOKENS_HEADER, encoding);
    if raw_response_cost.is_none() && raw_total_tokens.is_none() {
        return None;
    }
    Some(UpstreamReportedUsage {
        response_cost: parse_response_cost(raw_response_cost.as_deref()),
        total_tokens: parse_total_tokens(raw_total_tokens.as_deref()),
    })
}

#[must_use]
pub fn parse_response_cost(raw: Option<&str>) -> Option<f64> {
    let Some(raw_value) = raw else {
        tracing::warn!(
            "pass_through_endpoint: upstream did not send {UPSTREAM_RESPONSE_COST_HEADER}; recording 0 cost for this request"
        );
        return None;
    };
    let Ok(cost) = raw_value.trim().parse::<f64>() else {
        let shown = quoted_value(raw_value);
        tracing::warn!(
            "pass_through_endpoint: upstream sent unparseable {UPSTREAM_RESPONSE_COST_HEADER}={shown}; recording 0 cost for this request"
        );
        return None;
    };
    if !cost.is_finite() || cost < 0.0_f64 {
        let shown = quoted_value(raw_value);
        tracing::warn!(
            "pass_through_endpoint: upstream sent out-of-range {UPSTREAM_RESPONSE_COST_HEADER}={shown}; recording 0 cost for this request"
        );
        return None;
    }
    Some(cost)
}

#[must_use]
pub fn parse_total_tokens(raw: Option<&str>) -> Option<i64> {
    let Some(raw_value) = raw else {
        tracing::warn!(
            "pass_through_endpoint: upstream did not send {UPSTREAM_TOTAL_TOKENS_HEADER}; recording 0 tokens for this request"
        );
        return None;
    };
    let Ok(tokens) = raw_value.trim().parse::<i64>() else {
        let shown = quoted_value(raw_value);
        tracing::warn!(
            "pass_through_endpoint: upstream sent unparseable {UPSTREAM_TOTAL_TOKENS_HEADER}={shown}; recording 0 tokens for this request"
        );
        return None;
    };
    if tokens < 0 {
        let shown = quoted_value(raw_value);
        tracing::warn!(
            "pass_through_endpoint: upstream sent negative {UPSTREAM_TOTAL_TOKENS_HEADER}={shown}; recording 0 tokens for this request"
        );
        return None;
    }
    Some(tokens)
}

pub fn apply_upstream_reported_usage(
    state: &mut PassThroughCallState,
    headers: &HeaderMap,
) -> Option<UpstreamReportedUsage> {
    let reported = parse_upstream_reported_usage(headers)?;
    state.upstream_reported_usage = Some(reported);
    if let Some(cost) = reported.response_cost {
        state.response_cost = Some(cost);
    }
    if let Some(tokens) = reported.total_tokens {
        state.combined_usage_total_tokens = Some(tokens);
    }
    Some(reported)
}

#[must_use]
pub const fn has_upstream_reported_usage(state: &PassThroughCallState) -> bool {
    // the marker decides, even when both values it carried were unusable
    state.upstream_reported_usage.is_some()
}

// Python's %r of a str wraps it in single quotes and escapes the quote and the backslash
fn quoted_value(raw: &str) -> String {
    let mut repr = String::with_capacity(raw.len().saturating_add(2));
    repr.push('\'');
    for character in raw.chars() {
        match character {
            '\\' => repr.push_str("\\\\"),
            '\'' => repr.push_str("\\'"),
            '\n' => repr.push_str("\\n"),
            '\r' => repr.push_str("\\r"),
            '\t' => repr.push_str("\\t"),
            control if control.is_control() => push_control_escape(&mut repr, control),
            other => repr.push(other),
        }
    }
    repr.push('\'');
    repr
}

fn push_control_escape(repr: &mut String, character: char) {
    let code = u32::from(character);
    if code <= 0xFF {
        let escape = format!("\\x{code:02x}");
        repr.push_str(&escape);
    } else if code <= 0xFFFF {
        let escape = format!("\\u{code:04x}");
        repr.push_str(&escape);
    } else {
        let escape = format!("\\U{code:08x}");
        repr.push_str(&escape);
    }
}
