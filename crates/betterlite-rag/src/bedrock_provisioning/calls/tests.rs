#![cfg(test)]

use super::CallRequest;

fn request() -> CallRequest {
    CallRequest {
        operation: "GetCallerIdentity",
        service: "sts",
        signing_region: "us-west-2".to_owned(),
        method: "POST",
        url: "https://sts.us-west-2.amazonaws.com/".to_owned(),
        headers: vec![(
            "Authorization".to_owned(),
            "AWS4-HMAC-SHA256 credential".to_owned(),
        )],
        body: b"Action=GetCallerIdentity".to_vec(),
    }
}

#[test]
fn call_request_debug_names_the_headers_and_sizes_the_body() {
    let printed = format!("{:?}", request());
    assert!(printed.contains("header_names"));
    assert!(printed.contains("Authorization"));
    assert!(printed.contains("body_length: 24"));
    assert!(!printed.contains("Action=GetCallerIdentity"));
    assert!(!printed.contains("AWS4-HMAC-SHA256"));
}
