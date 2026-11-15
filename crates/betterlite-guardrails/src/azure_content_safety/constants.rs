pub const MAX_TEXT_LENGTH: usize = 10_000;

// Azure bills text in 1,000-character records, so a submitted chunk spends ceil(len / 1000) of them.
pub const TEXT_RECORD_LENGTH: usize = 1_000;
