#![cfg(test)]

use super::{RequestContext, RequestIdentity};

#[test]
fn identity_and_context_reexports_resolve() {
    let identity: Option<RequestIdentity> = None;
    let context: Option<RequestContext> = None;
    drop(identity);
    drop(context);
}
