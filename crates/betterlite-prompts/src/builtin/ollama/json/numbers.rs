pub(super) fn line(source: &str) -> String {
    let (body, comma) = source
        .strip_suffix(',')
        .map_or((source, ""), |body| (body, ","));
    let token = body
        .rsplit_once(": ")
        .map_or_else(|| body.trim_start(), |(_, value)| value);
    let Some(normalized) = number(token) else {
        return source.to_owned();
    };
    let prefix = body.strip_suffix(token).unwrap_or_default();
    format!("{prefix}{normalized}{comma}")
}

fn number(token: &str) -> Option<String> {
    if token == "-0" {
        return Some("0".to_owned());
    }
    if !token.contains(['.', 'e', 'E']) {
        return None;
    }
    let value: f64 = token.parse().ok()?;
    let shortest = format!("{value:?}");
    Some(match shortest.as_str() {
        "inf" => "Infinity".to_owned(),
        "-inf" => "-Infinity".to_owned(),
        _ => match shortest.split_once('e') {
            Some((mantissa, exponent)) => {
                let (sign, digits) = exponent
                    .strip_prefix('-')
                    .map_or(("+", exponent), |digits| ("-", digits));
                format!("{mantissa}e{sign}{digits:0>2}")
            }
            None => shortest,
        },
    })
}
