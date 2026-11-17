#![cfg(test)]
#![cfg_attr(
    test,
    expect(
        clippy::default_numeric_fallback,
        clippy::too_many_lines,
        reason = "the fixtures spell out the Soniox wire values and one test walks the whole job"
    )
)]

mod failures;
mod formats;
mod references;
mod selection;
mod support;
