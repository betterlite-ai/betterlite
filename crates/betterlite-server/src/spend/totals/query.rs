use std::collections::BTreeMap;

const LIMIT_PARAM: &str = "limit";
const API_KEY_PARAM: &str = "api_key";

#[derive(Clone, Default, Eq, PartialEq)]
pub struct SpendTotalsQuery {
    pub api_key: Option<String>,
    pub limit: LimitQuery,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum LimitQuery {
    #[default]
    Absent,
    Value(i64),
    Invalid,
}

#[must_use]
pub fn decode_spend_totals_query(raw: &[u8]) -> SpendTotalsQuery {
    let values: BTreeMap<String, String> = form_urlencoded::parse(raw).into_owned().collect();
    let limit = values.get(LIMIT_PARAM).map_or(LimitQuery::Absent, |text| {
        parse_limit(text).map_or(LimitQuery::Invalid, LimitQuery::Value)
    });
    SpendTotalsQuery {
        api_key: values.get(API_KEY_PARAM).cloned(),
        limit,
    }
}

// pydantic reads a JSON int from text: digits with single underscores, an optional sign, and a zero fraction
fn parse_limit(text: &str) -> Option<i64> {
    let trimmed = text.trim();
    let (negative, unsigned) = trimmed.strip_prefix('-').map_or_else(
        || (false, trimmed.strip_prefix('+').unwrap_or(trimmed)),
        |rest| (true, rest),
    );
    let (whole, fraction) = match unsigned.split_once('.') {
        Some((whole, fraction)) => (whole, Some(fraction)),
        None => (unsigned, None),
    };
    if let Some(decimals) = fraction
        && (decimals.is_empty() || !decimals.bytes().all(|byte| byte == b'0'))
    {
        return None;
    }
    let magnitude = i128::try_from(parse_digits(whole)?).ok()?;
    let value = if negative {
        magnitude.checked_neg()?
    } else {
        magnitude
    };
    i64::try_from(value).ok()
}

fn parse_digits(digits: &str) -> Option<u128> {
    let mut magnitude = 0_u128;
    let mut after_digit = false;
    for byte in digits.bytes() {
        if byte.is_ascii_digit() {
            let digit = u128::from(byte).checked_sub(u128::from(b'0'))?;
            magnitude = magnitude.checked_mul(10)?.checked_add(digit)?;
            after_digit = true;
        } else if byte == b'_' && after_digit {
            after_digit = false;
        } else {
            return None;
        }
    }
    after_digit.then_some(magnitude)
}
