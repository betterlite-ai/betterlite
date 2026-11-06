#![no_main]

use std::str;

use betterlite_http::user_url::address::is_blocked_text;
use betterlite_http::user_url::httpx_url::HttpxUrl;
use betterlite_http::user_url::join::urljoin;
use betterlite_http::user_url::redirect::redirect_url;
use betterlite_http::user_url::split::urlparse;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let Ok(text) = str::from_utf8(bytes.get(..4096).unwrap_or(bytes)) else {
        return;
    };
    let (base, location) = text.split_once('\n').unwrap_or((text, ""));
    let _blocked = is_blocked_text(base);
    if let Ok(parsed) = urlparse(base, "") {
        let _host = (parsed.hostname(), parsed.port());
        let _unparsed = parsed.unparse();
    }
    let _joined = urljoin(base, location);
    if let Ok(request) = HttpxUrl::parse(base) {
        let _redirect = redirect_url(&request, location);
    }
});
