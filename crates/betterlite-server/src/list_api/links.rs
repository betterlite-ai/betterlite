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
    PageLinks {
        self_link: page_url(path, pairs, page),
        prev: (page > 1).then(|| page_url(path, pairs, page.saturating_sub(1))),
        next: has_more.then(|| page_url(path, pairs, page.saturating_add(1))),
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
    ListLinks {
        self_link: page_url(path, pairs, page),
        first: page_url(path, pairs, 1),
        prev: (page > 1).then(|| page_url(path, pairs, page.saturating_sub(1))),
        next: (page < last).then(|| page_url(path, pairs, page.saturating_add(1))),
        last: page_url(path, pairs, last),
    }
}
