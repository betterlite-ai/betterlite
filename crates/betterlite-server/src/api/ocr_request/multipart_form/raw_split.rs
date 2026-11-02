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

// Splits off the next CRLF-terminated line, returning the line and what follows its CRLF.
fn next_line(cursor: &[u8]) -> Option<(&[u8], &[u8])> {
    let idx = cursor.windows(2).position(|pair| pair == b"\r\n")?;
    let (line, after_line) = cursor.split_at(idx);
    Some((line, after_line.get(2..).unwrap_or(&[])))
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn is_final_boundary(line: &[u8], dash_boundary: &[u8]) -> bool {
    line.strip_prefix(dash_boundary)
        .is_some_and(|rest| rest.starts_with(b"--"))
}

fn parse_header_line(line: &[u8]) -> Result<(&[u8], &[u8]), OcrRequestError> {
    let separator = find_subslice(line, b": ").ok_or(OcrRequestError::MultipartParse(
        MultipartError::InvalidItemHeader,
    ))?;
    let (name, rest) = line.split_at(separator);
    let value = rest.get(2..).unwrap_or(&[]);
    Ok((name, value))
}

// Scans past everything before the first boundary line (the preamble, which MIME allows and ignores), returning what follows its trailing CRLF.
fn skip_preamble<'body>(body: &'body [u8], dash_boundary: &[u8]) -> &'body [u8] {
    let mut remaining = body;
    while let Some((line, after_crlf)) = next_line(remaining) {
        if line.starts_with(dash_boundary) {
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
    while let Some((line, after_crlf)) = next_line(cursor) {
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
    while let Some((line, after_crlf)) = next_line(cursor) {
        if line.starts_with(dash_boundary) {
            let consumed = remaining.len().saturating_sub(cursor.len());
            let content_len = consumed.saturating_sub(2);
            let (content, _) = remaining.split_at(content_len);
            return (content, after_crlf, is_final_boundary(line, dash_boundary));
        }
        cursor = after_crlf;
    }
    (remaining, &[], true)
}

// A byte-level split of the multipart body that never requires a header value to be valid UTF-8, unlike multipart-rs's own streaming reader used in parse_body for the boundary and Content-Type.
pub(super) fn split_items<'body>(
    body: &'body [u8],
    boundary: &str,
) -> Result<Vec<RawItem<'body>>, OcrRequestError> {
    let dash_boundary = format!("--{boundary}").into_bytes();
    let mut items = Vec::new();
    let mut remaining = skip_preamble(body, &dash_boundary);
    loop {
        let headers = read_headers(remaining)?;
        let (data, after_data, is_final) = read_data(headers.after_headers, &dash_boundary);
        items.push(RawItem {
            content_disposition: headers.content_disposition,
            content_type: headers.content_type,
            data,
        });
        if is_final {
            return Ok(items);
        }
        remaining = after_data;
    }
}
