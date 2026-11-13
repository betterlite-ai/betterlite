use std::collections::BTreeSet;

// LiteLLM's IN_LIST_CHUNK_SIZE keeps one statement well under Postgres's bind-parameter cap
pub const EMAIL_PAGE_SIZE: usize = 5_000;

pub(crate) fn email_pages(user_emails: &[String]) -> Vec<Vec<String>> {
    let unique: BTreeSet<&str> = user_emails.iter().map(String::as_str).collect();
    let mut pages = Vec::new();
    let mut page = Vec::new();
    for value in unique {
        page.push(value.to_owned());
        if page.len() == EMAIL_PAGE_SIZE {
            pages.push(page);
            page = Vec::new();
        }
    }
    if !page.is_empty() {
        pages.push(page);
    }
    pages
}
