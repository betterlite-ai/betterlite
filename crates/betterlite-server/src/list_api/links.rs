use crate::list_api::plan::types::PAGE_PARAM;
use crate::management_envelopes::{ListLinks, PageLinks};
mod tests;

fn page_url(path: &str, pairs: &[(String, String)], page: u64) -> String {
    let mut serializer = form_urlencoded::Serializer::new(String::new());
    for (key, value) in pairs.iter().filter(|(key, _)| key != PAGE_PARAM) {
        serializer.append_pair(key, value);
    }
    serializer.append_pair(PAGE_PARAM, &page.to_string());
    format!("{path}?{}", serializer.finish())
}

#[must_use]
pub fn build_page_links(
    path: &str,
    pairs: &[(String, String)],
    page: u64,
    has_more: bool,
) -> PageLinks {
    let previous_page = page.saturating_sub(1);
    let next_page = page.saturating_add(1);
    let prev = (page > 1).then(|| page_url(path, pairs, previous_page));
    let next = has_more.then(|| page_url(path, pairs, next_page));
    PageLinks {
        self_link: page_url(path, pairs, page),
        prev,
        next,
    }
}

#[must_use]
pub fn build_list_links(
    path: &str,
    pairs: &[(String, String)],
    page: u64,
    total_pages: u64,
) -> ListLinks {
    let last = total_pages.max(1);
    let first_link = page_url(path, pairs, 1);
    let last_link = page_url(path, pairs, last);
    let prev = (page > 1).then(|| page_url(path, pairs, page.saturating_sub(1)));
    let next = (page < last).then(|| page_url(path, pairs, page.saturating_add(1)));
    ListLinks {
        self_link: page_url(path, pairs, page),
        first: first_link,
        prev,
        next,
        last: last_link,
    }
}
