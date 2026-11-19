#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct CacheActivityQuery {
    pub(crate) start_date: String,
    pub(crate) end_date: String,
    pub(crate) key_aliases: Vec<String>,
    pub(crate) models: Vec<String>,
}

// Starlette keeps a scalar's last value and a list's values in order.
pub(crate) fn decode(raw_query: Option<&str>) -> Result<CacheActivityQuery, Vec<&'static str>> {
    let mut start_date = None;
    let mut end_date = None;
    let mut key_aliases = Vec::new();
    let mut models = Vec::new();
    for (name, value) in
        form_urlencoded::parse(raw_query.unwrap_or_default().as_bytes()).into_owned()
    {
        match name.as_str() {
            "start_date" => start_date = Some(value),
            "end_date" => end_date = Some(value),
            "key_aliases" => key_aliases.push(value),
            "models" => models.push(value),
            _ => {}
        }
    }
    let missing = missing_dates(start_date.as_deref(), end_date.as_deref());
    let (Some(start_value), Some(end_value)) = (start_date, end_date) else {
        return Err(missing);
    };
    Ok(CacheActivityQuery {
        start_date: start_value,
        end_date: end_value,
        key_aliases,
        models,
    })
}

fn missing_dates(start_date: Option<&str>, end_date: Option<&str>) -> Vec<&'static str> {
    let mut missing = Vec::new();
    if start_date.is_none() {
        missing.push("start_date");
    }
    if end_date.is_none() {
        missing.push("end_date");
    }
    missing
}
