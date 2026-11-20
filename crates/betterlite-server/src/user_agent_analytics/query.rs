use super::error::QueryIssue;
use super::error::QueryIssues;

const DEFAULT_PAGE: i64 = 1;
const DEFAULT_PAGE_SIZE: i64 = 50;
const MIN_PAGE: i64 = 1;
const MAX_PAGE_SIZE: i64 = 1000;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TagFilter {
    Substring(String),
    Exact(Vec<String>),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Pagination {
    pub page: i64,
    pub page_size: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AnalyticsQuery {
    tag_filter: Option<String>,
    tag_filters: Vec<String>,
    start_date: Option<String>,
    end_date: Option<String>,
    page: Option<String>,
    page_size: Option<String>,
}

impl AnalyticsQuery {
    #[must_use]
    pub fn decode(raw: &str) -> Self {
        let mut query = Self {
            tag_filter: None,
            tag_filters: Vec::new(),
            start_date: None,
            end_date: None,
            page: None,
            page_size: None,
        };
        for (name, value) in form_urlencoded::parse(raw.as_bytes()) {
            let key: &str = name.as_ref();
            match key {
                "tag_filter" => query.tag_filter = Some(value.into_owned()),
                "tag_filters" => query.tag_filters.push(value.into_owned()),
                "start_date" => query.start_date = Some(value.into_owned()),
                "end_date" => query.end_date = Some(value.into_owned()),
                "page" => query.page = Some(value.into_owned()),
                "page_size" => query.page_size = Some(value.into_owned()),
                _ => {}
            }
        }
        query
    }

    #[must_use]
    pub fn filter(&self) -> Option<TagFilter> {
        if !self.tag_filters.is_empty() {
            return Some(TagFilter::Exact(self.tag_filters.clone()));
        }
        self.tag_filter
            .as_ref()
            .map(|text| TagFilter::Substring(text.clone()))
    }

    pub fn date_range(&self) -> Result<(&str, &str), QueryIssues> {
        match (self.start_date.as_deref(), self.end_date.as_deref()) {
            (Some(start), Some(end)) => Ok((start, end)),
            (start, end) => {
                let mut issues = Vec::new();
                if start.is_none() {
                    issues.push(missing("start_date"));
                }
                if end.is_none() {
                    issues.push(missing("end_date"));
                }
                Err(QueryIssues { issues })
            }
        }
    }

    pub fn pagination(&self) -> Result<Pagination, QueryIssues> {
        let mut issues = Vec::new();
        let mut page = DEFAULT_PAGE;
        if let Some(text) = self.page.as_deref() {
            match bounded_value(text, "page", MIN_PAGE, i64::MAX) {
                Ok(value) => page = value,
                Err(issue) => issues.push(issue),
            }
        }
        let mut page_size = DEFAULT_PAGE_SIZE;
        if let Some(text) = self.page_size.as_deref() {
            match bounded_value(text, "page_size", MIN_PAGE, MAX_PAGE_SIZE) {
                Ok(value) => page_size = value,
                Err(issue) => issues.push(issue),
            }
        }
        if !issues.is_empty() {
            return Err(QueryIssues { issues });
        }
        Ok(Pagination { page, page_size })
    }
}

fn missing(name: &'static str) -> QueryIssue {
    QueryIssue {
        kind: "missing",
        loc: ["query", name],
        msg: "Field required".to_owned(),
    }
}

fn bounded_value(
    text: &str,
    name: &'static str,
    minimum: i64,
    maximum: i64,
) -> Result<i64, QueryIssue> {
    let Ok(value) = text.trim().parse::<i64>() else {
        return Err(QueryIssue {
            kind: "int_parsing",
            loc: ["query", name],
            msg: "Input should be a valid integer, unable to parse string as an integer".to_owned(),
        });
    };
    if value < minimum {
        return Err(QueryIssue {
            kind: "greater_than_equal",
            loc: ["query", name],
            msg: format!("Input should be greater than or equal to {minimum}"),
        });
    }
    if value > maximum {
        return Err(QueryIssue {
            kind: "less_than_equal",
            loc: ["query", name],
            msg: format!("Input should be less than or equal to {maximum}"),
        });
    }
    Ok(value)
}
