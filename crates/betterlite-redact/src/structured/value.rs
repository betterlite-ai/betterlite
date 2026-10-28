use super::{REDACTED, SensitiveDataMasker};

pub(super) fn mask(masker: &SensitiveDataMasker, text: &str) -> String {
    let length = text.chars().count();
    let visible = masker.visible_prefix.saturating_add(masker.visible_suffix);
    if length <= visible && !masker.mask_short_values {
        return text.to_owned();
    }
    let hidden = if length <= visible {
        length
    } else {
        length.saturating_sub(visible)
    };
    if hidden
        .checked_mul(masker.mask_char.len())
        .is_none_or(|bytes| bytes > masker.max_work)
    {
        return REDACTED.to_owned();
    }
    let replacement = masker.mask_char.repeat(hidden);
    if length <= visible {
        return replacement;
    }
    let prefix = text.chars().take(masker.visible_prefix).collect::<String>();
    let suffix = text
        .chars()
        .skip(length.saturating_sub(masker.visible_suffix))
        .collect::<String>();
    let output_bytes = prefix
        .len()
        .saturating_add(replacement.len())
        .saturating_add(suffix.len());
    if output_bytes > masker.max_work {
        REDACTED.to_owned()
    } else {
        format!("{prefix}{replacement}{suffix}")
    }
}

mod tests;
