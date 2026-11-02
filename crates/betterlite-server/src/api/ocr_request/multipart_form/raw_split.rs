use super::OcrRequestError;
use multipart_rs::MultipartError;

pub(super) struct RawItem<'body> {
    pub(super) content_disposition: Option<&'body [u8]>,
    pub(super) content_type: Option<&'body [u8]>,
    pub(super) data: &'body [u8],
}

enum HeaderKind {
    ContentDisposition,
    ContentType,
    Other,
}

const fn header_kind(name: &[u8]) -> HeaderKind {
    if name.eq_ignore_ascii_case(b"content-disposition") {
        HeaderKind::ContentDisposition
    } else if name.eq_ignore_ascii_case(b"content-type") {
        HeaderKind::ContentType
    } else {
        HeaderKind::Other
    }
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

// A line is a boundary only when dash_boundary is immediately followed by nothing (an ordinary boundary) or by "--" (the closing boundary, which needs no trailing CRLF since it can end the body). Anything else, such as a data line that merely starts with dash_boundary, is not a boundary at all.
fn boundary_kind(line: &[u8], dash_boundary: &[u8]) -> Option<bool> {
    let rest = line.strip_prefix(dash_boundary)?;
    if rest.is_empty() {
        Some(false)
    } else if rest == b"--" {
        Some(true)
    } else {
        None
    }
}

fn parse_header_line(line: &[u8]) -> Result<(&[u8], &[u8]), OcrRequestError> {
    let separator = find_subslice(line, b": ").ok_or(OcrRequestError::MultipartParse(
        MultipartError::InvalidItemHeader,
    ))?;
    let (name, rest) = line.split_at(separator);
    let value = rest.get(2..).unwrap_or(&[]);
    Ok((name, value))
}

// Scans past everything before the first boundary line (the preamble, which MIME allows and ignores), returning what follows its trailing CRLF. Each loop below finds only the CRLF's offset, not a slice pair: a caller advancing by that offset plus a literal 2 always shrinks its own cursor, so a mutated offset can at worst spin without allocating, not leak memory on every turn.
fn skip_preamble<'body>(body: &'body [u8], dash_boundary: &[u8]) -> &'body [u8] {
    let mut remaining = body;
    while let Some(idx) = remaining.windows(2).position(|pair| pair == b"\r\n") {
        let line = remaining.get(..idx).unwrap_or(remaining);
        let after_crlf = remaining.get(idx.saturating_add(2)..).unwrap_or(&[]);
        if boundary_kind(line, dash_boundary).is_some() {
            return after_crlf;
        }
        remaining = after_crlf;
    }
    &[]
}

struct PartHeaders<'body> {
    content_disposition: Option<&'body [u8]>,
    content_type: Option<&'body [u8]>,
    after_headers: &'body [u8],
}

// Reads header lines up to the blank line that ends a part's header block, returning the Content-Disposition and Content-Type values found and what follows the blank line.
fn read_headers(remaining: &[u8]) -> Result<PartHeaders<'_>, OcrRequestError> {
    let mut content_disposition = None;
    let mut content_type = None;
    let mut cursor = remaining;
    while let Some(idx) = cursor.windows(2).position(|pair| pair == b"\r\n") {
        let line = cursor.get(..idx).unwrap_or(cursor);
        let after_crlf = cursor.get(idx.saturating_add(2)..).unwrap_or(&[]);
        if line.trim_ascii().is_empty() {
            return Ok(PartHeaders {
                content_disposition,
                content_type,
                after_headers: after_crlf,
            });
        }
        let (name, value) = parse_header_line(line)?;
        match header_kind(name) {
            HeaderKind::ContentDisposition => content_disposition = Some(value),
            HeaderKind::ContentType => content_type = Some(value),
            HeaderKind::Other => {}
        }
        cursor = after_crlf;
    }
    Ok(PartHeaders {
        content_disposition,
        content_type,
        after_headers: &[],
    })
}

// Reads a part's body up to the next boundary line, returning the content (without its trailing CRLF), what follows the boundary line's CRLF, and whether it was the closing boundary.
fn read_data<'body>(
    remaining: &'body [u8],
    dash_boundary: &[u8],
) -> (&'body [u8], &'body [u8], bool) {
    let mut cursor = remaining;
    loop {
        if let Some(idx) = cursor.windows(2).position(|pair| pair == b"\r\n") {
            let line = cursor.get(..idx).unwrap_or(cursor);
            let after_crlf = cursor.get(idx.saturating_add(2)..).unwrap_or(&[]);
            if let Some(is_final) = boundary_kind(line, dash_boundary) {
                let consumed = remaining.len().saturating_sub(cursor.len());
                let content_len = consumed.saturating_sub(2);
                let content = remaining.get(..content_len).unwrap_or(remaining);
                return (content, after_crlf, is_final);
            }
            cursor = after_crlf;
        } else if let Some(is_final) = boundary_kind(cursor, dash_boundary) {
            // The body can end right after the closing boundary's second hyphen, with no trailing CRLF, so a final unterminated line still needs checking before falling back to treating the rest as content.
            let consumed = remaining.len().saturating_sub(cursor.len());
            let content_len = consumed.saturating_sub(2);
            let content = remaining.get(..content_len).unwrap_or(remaining);
            return (content, &[], is_final);
        } else {
            return (remaining, &[], true);
        }
    }
}

// A byte-level split of the multipart body that never requires a header value to be valid UTF-8, unlike multipart-rs's own streaming reader used in parse_body for the boundary and Content-Type. Bailing once the item count passes max_items keeps a body of many tiny parts from growing the item list well past parse_body's own field/file caps before those caps ever run.
pub(super) fn split_items<'body>(
    body: &'body [u8],
    boundary: &str,
    max_items: usize,
) -> Result<Vec<RawItem<'body>>, OcrRequestError> {
    let dash_boundary = format!("--{boundary}").into_bytes();
    let mut items = Vec::new();
    let mut remaining = skip_preamble(body, &dash_boundary);
    loop {
        let before = remaining.len();
        let headers = read_headers(remaining)?;
        let (data, after_data, is_final) = read_data(headers.after_headers, &dash_boundary);
        items.push(RawItem {
            content_disposition: headers.content_disposition,
            content_type: headers.content_type,
            data,
        });
        if items.len() > max_items {
            return Err(OcrRequestError::MultipartParse(
                MultipartError::InvalidItemHeader,
            ));
        }
        // Legitimate parsing always shrinks remaining by at least the boundary line it consumed, so this stops the loop on a mutant that fakes is_final instead of trusting it alone.
        if is_final || after_data.len() >= before {
            return Ok(items);
        }
        remaining = after_data;
    }
}
