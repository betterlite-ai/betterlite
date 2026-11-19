#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TagFilter {
    Substring(String),
    Exact(Vec<String>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActiveUsersQuery {
    tag_filter: Option<String>,
    tag_filters: Vec<String>,
}

impl ActiveUsersQuery {
    pub fn decode(raw: &str) -> Self {
        let mut tag_filter = None;
        let mut tag_filters = Vec::new();
        for (name, value) in form_urlencoded::parse(raw.as_bytes()) {
            let key: &str = name.as_ref();
            match key {
                "tag_filter" => tag_filter = Some(value.into_owned()),
                "tag_filters" => tag_filters.push(value.into_owned()),
                _ => {}
            }
        }
        Self {
            tag_filter,
            tag_filters,
        }
    }

    pub fn filter(&self) -> Option<TagFilter> {
        if !self.tag_filters.is_empty() {
            return Some(TagFilter::Exact(self.tag_filters.clone()));
        }
        self.tag_filter
            .as_ref()
            .map(|text| TagFilter::Substring(text.clone()))
    }
}
