#![cfg(test)]
#![cfg_attr(
    test,
    expect(
        clippy::too_many_lines,
        reason = "the speech fixtures spell out whole credential chains and provider request bodies"
    )
)]

mod branches;
mod credentials;
mod defaults;
mod errors;
mod logging;
mod support;
mod vertex;
