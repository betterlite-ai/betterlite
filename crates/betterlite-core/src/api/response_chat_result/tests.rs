#![cfg(test)]
#![cfg_attr(
    test,
    expect(
        clippy::default_numeric_fallback,
        clippy::disallowed_methods,
        reason = "json! fixtures state expected provider JSON directly"
    )
)]

mod choices;
mod support;
mod transform;
