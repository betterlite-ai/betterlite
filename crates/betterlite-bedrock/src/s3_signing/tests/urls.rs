use crate::s3_signing::file_object::convert_https_url_to_s3_uri;

fn assert_converted(cases: &[(&str, &str)]) {
    for (url, expected) in cases {
        let converted = match convert_https_url_to_s3_uri(url) {
            Ok((uri, filename)) => format!("{uri} | {filename}"),
            Err(error) => format!("error: {error}"),
        };
        assert_eq!(converted, *expected, "{url}");
    }
}

pub(super) fn s3_virtual_and_path_style() {
    regex_matches();
    path_fallback();
    unparsable_urls();
}

fn regex_matches() {
    assert_converted(&[
        (
            "https://s3.us-west-2.amazonaws.com/b/k%20x?q#f",
            "s3://b/k x?q#f | k x?q#f",
        ),
        (
            "https://b.s3.us-west-2.amazonaws.com/a/b/",
            "s3://b/a/b/ | ",
        ),
        (
            "https://s3.r.amazonaws.com/b/d/%2541\nz",
            "s3://b/d/%41 | %41",
        ),
        (
            "https://my-b.s3.r.amazonaws.com/k.jsonl",
            "s3://my-b/k.jsonl | k.jsonl",
        ),
    ]);
}

fn path_fallback() {
    assert_converted(&[
        // the regexes are case sensitive, so the fallback drops the query
        ("https://S3.us-west-2.amazonaws.com/b/k?q", "s3://b/k | k"),
        ("https://s3.cn-north-1.amazonaws.com.cn/b/k", "s3://b/k | k"),
        (
            "http://localhost:9000//bkt/dir/f.jsonl?sig=1#x",
            "s3://bkt/dir/f.jsonl | f.jsonl",
        ),
        ("https://s3.r.amazonaws.com/b/", "s3://b/ | "),
        ("https://h/b/k;v=1", "s3://b/k | k"),
        ("s3x://h/b/k;v=1", "s3://b/k;v=1 | k;v=1"),
        ("  https://h/b/\tk", "s3://b/k | k"),
        ("b/k", "s3://b/k | k"),
        // urlparse accepts a bracketed IPv6 host and keeps its path
        ("https://[::1]/b/k", "s3://b/k | k"),
        // a key that starts with a newline gives the regexes nothing to capture
        ("https://s3.r.amazonaws.com/b/\nx", "s3://b/x | x"),
        ("https://bk.s3.r.amazonaws.com/\nx/y", "s3://x/y | y"),
        // an empty region fails both regexes, and the fallback drops the query
        ("https://s3..amazonaws.com/b/k?q", "s3://b/k | k"),
        (
            "https://b.s3..amazonaws.com/k?q",
            "error: Unable to parse S3 URL: https://b.s3..amazonaws.com/k?q",
        ),
        // a scheme must start with a letter, so 1x: stays part of the path
        ("1x://h/b/k", "s3://1x://h/b/k | k"),
    ]);
}

fn unparsable_urls() {
    let urls = [
        "https://s3.r.amazonaws.com//k",
        "https://.s3.r.amazonaws.com/k",
        "https://x/onlybucket",
        "https://my.bucket.s3.us-west-2.amazonaws.com/k",
        "https://[h/b/k",
        "https://h]/b/k",
        // urlparse rejects a bracketed host that is not an IPv6 address
        "https://[h]/b/k",
        "https://[127.0.0.1]/b/k",
        // urlparse rejects a netloc whose NFKC form contains a path separator
        "https://\u{2100}/b/k",
    ];
    for url in urls {
        let error = convert_https_url_to_s3_uri(url).unwrap_err();
        assert_eq!(error.to_string(), format!("Unable to parse S3 URL: {url}"));
    }
}
