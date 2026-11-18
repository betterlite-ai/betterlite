use multipart_rs::MultipartError;

pub(crate) struct RawItem<'body> {
    pub(crate) content_disposition: Option<&'body [u8]>,
    pub(crate) content_type: Option<&'body [u8]>,
    pub(crate) data: &'body [u8],
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

// A line is a boundary only when dash_boundary is immediately followed by nothing (an ordinary boundary) or by "--" (the closing boundary). python-multipart stops at the second hyphen and drops everything else on that line, so a closing boundary followed by trailing spaces or an epilogue (no trailing CRLF needed, since it can end the body) is still the closing boundary. Anything else, such as a data line that merely starts with dash_boundary, is not a boundary at all.
fn boundary_kind(line: &[u8], dash_boundary: &[u8]) -> Option<bool> {
    let rest = line.strip_prefix(dash_boundary)?;
    if rest.starts_with(b"--") {
        Some(true)
    } else if rest.is_empty() {
        Some(false)
    } else {
        None
    }
}

// python-multipart's HEADER_FIELD ends at the first colon and HEADER_VALUE_START then skips zero or more spaces, so a header whose colon has no following space is still split into a name and a value.
fn parse_header_line(line: &[u8]) -> Result<(&[u8], &[u8]), MultipartError> {
    let separator = line
        .iter()
        .position(|byte| *byte == b':')
        .ok_or(MultipartError::InvalidItemHeader)?;
    let (name, rest) = line.split_at(separator);
    let mut value = rest.get(1..).unwrap_or(&[]);
    while let Some(stripped) = value.strip_prefix(b" ") {
        value = stripped;
    }
    Ok((name, value))
}

enum BoundaryScan<'body> {
    Found {
        content: &'body [u8],
        after: &'body [u8],
        is_final: bool,
    },
    NoBoundary,
}

// Scans line by line for the first boundary line and returns the part content before it. A boundary on the very first line counts only when allow_first is set, so a part whose data starts with a boundary line keeps it as content, and the closing delimiter can end the body with no trailing CRLF. Each pass advances by the found offset plus the CRLF width, and a body.get that runs past the end stops the scan, so a mutated offset cannot loop.
fn scan_boundary<'body>(
    body: &'body [u8],
    dash_boundary: &[u8],
    allow_first: bool,
) -> BoundaryScan<'body> {
    let mut start = 0_usize;
    loop {
        let Some(cursor) = body.get(start..) else {
            return BoundaryScan::NoBoundary;
        };
        let Some(offset) = cursor.windows(2).position(|pair| pair == b"\r\n") else {
            let Some(is_final) =
                boundary_kind(cursor, dash_boundary).filter(|_| allow_first || start > 0)
            else {
                return BoundaryScan::NoBoundary;
            };
            return BoundaryScan::Found {
                content: body.get(..start.saturating_sub(2)).unwrap_or(&[]),
                after: &[],
                is_final,
            };
        };
        let end = start.saturating_add(offset);
        let line = body.get(start..end).unwrap_or(&[]);
        if let Some(is_final) =
            boundary_kind(line, dash_boundary).filter(|_| allow_first || start > 0)
        {
            return BoundaryScan::Found {
                content: body.get(..start.saturating_sub(2)).unwrap_or(&[]),
                after: body.get(end.saturating_add(2)..).unwrap_or(&[]),
                is_final,
            };
        }
        start = end.saturating_add(2);
    }
}

// Scans past everything before the first boundary line (the preamble, which MIME allows and ignores), returning what follows its trailing CRLF and whether that line already closed the message. A body with no boundary line at all returns None, which split_items reports as malformed.
fn skip_preamble<'body>(body: &'body [u8], dash_boundary: &[u8]) -> Option<(&'body [u8], bool)> {
    match scan_boundary(body, dash_boundary, true) {
        BoundaryScan::Found {
            after, is_final, ..
        } => Some((after, is_final)),
        BoundaryScan::NoBoundary => None,
    }
}

struct PartHeaders<'body> {
    content_disposition: Option<&'body [u8]>,
    content_type: Option<&'body [u8]>,
    after_headers: &'body [u8],
}

// Reads header lines up to the blank line that ends a part's header block, returning the Content-Disposition and Content-Type values found and what follows the blank line.
fn read_headers(remaining: &[u8]) -> Result<PartHeaders<'_>, MultipartError> {
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

// Reads a part's body up to the next boundary line, returning the content (without its trailing CRLF), what follows the boundary line's CRLF, and whether it was the closing boundary. A boundary on the data's first line is content, and a body with no boundary at all is all content.
fn read_data<'body>(
    remaining: &'body [u8],
    dash_boundary: &[u8],
) -> (&'body [u8], &'body [u8], bool) {
    match scan_boundary(remaining, dash_boundary, false) {
        BoundaryScan::Found {
            content,
            after,
            is_final,
        } => (content, after, is_final),
        BoundaryScan::NoBoundary => (remaining, &[], true),
    }
}

pub(crate) struct SplitItems<'body> {
    dash_boundary: Vec<u8>,
    remaining: &'body [u8],
    done: bool,
}

impl<'body> Iterator for SplitItems<'body> {
    type Item = Result<RawItem<'body>, MultipartError>;

    // Reads one item at a time rather than collecting every part into a Vec first, so a caller's own per-item cap (parse_body's field and file counts) runs before the next part is read, the way python-multipart's own streaming reader counts as it reads.
    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        let before = self.remaining.len();
        let headers = match read_headers(self.remaining) {
            Ok(headers) => headers,
            Err(error) => {
                self.done = true;
                return Some(Err(error));
            }
        };
        let (data, after_data, is_final) = read_data(headers.after_headers, &self.dash_boundary);
        // Legitimate parsing always shrinks remaining by at least the boundary line it consumed, so this stops iteration on a mutant that fakes is_final instead of trusting it alone.
        self.done = is_final || after_data.len() >= before;
        self.remaining = after_data;
        Some(Ok(RawItem {
            content_disposition: headers.content_disposition,
            content_type: headers.content_type,
            data,
        }))
    }
}

// A byte-level split of the multipart body that never requires a header value to be valid UTF-8, unlike multipart-rs's own streaming reader used in parse_body for the boundary and Content-Type. An immediate closing delimiter ends the message with no items, while a body with no boundary line at all is an error.
pub(crate) fn split_items<'body>(
    body: &'body [u8],
    boundary: &str,
) -> Result<SplitItems<'body>, MultipartError> {
    let dash_boundary = format!("--{boundary}").into_bytes();
    let Some((remaining, closed)) = skip_preamble(body, &dash_boundary) else {
        return Err(MultipartError::InvalidBoundary);
    };
    Ok(SplitItems {
        dash_boundary,
        remaining,
        done: closed,
    })
}
