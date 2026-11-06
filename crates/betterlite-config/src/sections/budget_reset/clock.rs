use chrono::NaiveTime;

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum ClockError {
    #[error("budget_reset_time must be a 24-hour 'HH:MM' or 'HH:MM:SS' string, e.g. \"12:00\"")]
    Malformed,
}

// Unicode 15.1's decimal digit blocks, the version the pinned Python resolves \d against; each block runs ten digits from zero
const DECIMAL_ZEROS: &str = concat!(
    "\u{30}\u{660}\u{6f0}\u{7c0}\u{966}\u{9e6}\u{a66}\u{ae6}\u{b66}\u{be6}\u{c66}\u{ce6}\u{d66}",
    "\u{de6}\u{e50}\u{ed0}\u{f20}\u{1040}\u{1090}\u{17e0}\u{1810}\u{1946}\u{19d0}\u{1a80}\u{1a90}",
    "\u{1b50}\u{1bb0}\u{1c40}\u{1c50}\u{a620}\u{a8d0}\u{a900}\u{a9d0}\u{a9f0}\u{aa50}\u{abf0}",
    "\u{ff10}\u{104a0}\u{10d30}\u{11066}\u{110f0}\u{11136}\u{111d0}\u{112f0}\u{11450}\u{114d0}",
    "\u{11650}\u{116c0}\u{11730}\u{118e0}\u{11950}\u{11c50}\u{11d50}\u{11da0}\u{11f50}\u{16a60}",
    "\u{16ac0}\u{16b50}\u{1d7ce}\u{1d7d8}\u{1d7e2}\u{1d7ec}\u{1d7f6}\u{1e140}\u{1e2f0}\u{1e4f0}",
    "\u{1e950}\u{1fbf0}",
);

#[derive(Clone, Copy)]
enum ClockField {
    Hour,
    Minute,
    Second,
}

pub fn decode_reset_clock(text: &str) -> Result<NaiveTime, ClockError> {
    let mut parts = text.split(':');
    let hour = parts
        .next()
        .and_then(|part| clock_field(part, ClockField::Hour));
    let minute = parts
        .next()
        .and_then(|part| clock_field(part, ClockField::Minute));
    let second = parts
        .next()
        .map_or(Some(0), |part| clock_field(part, ClockField::Second));
    if parts.next().is_some() {
        return Err(ClockError::Malformed);
    }
    let (Some(hour_value), Some(minute_value), Some(second_value)) = (hour, minute, second) else {
        return Err(ClockError::Malformed);
    };
    // Python's %H and %M reach chrono's parser only after this normalization
    let normalized = format!("{hour_value:02}:{minute_value:02}:{second_value:02}");
    NaiveTime::parse_from_str(&normalized, "%H:%M:%S").map_err(|_source| ClockError::Malformed)
}

fn clock_field(part: &str, field: ClockField) -> Option<u8> {
    let mut characters = part.chars();
    match (characters.next(), characters.next(), characters.next()) {
        (Some(single), None, _) => decimal_value(single),
        (Some(lead), Some(tail), None) => two_digit_value(lead, tail, field),
        _ => None,
    }
}

fn two_digit_value(lead: char, tail: char, field: ClockField) -> Option<u8> {
    let tens: u8 = match field {
        // %H admits a non-ASCII second digit only after the ASCII digit 0 or 1
        ClockField::Hour if matches!(lead, '0' | '1') => u8::try_from(lead.to_digit(10)?).ok()?,
        ClockField::Hour if lead == '2' && matches!(tail, '0'..='3') => 2,
        ClockField::Minute | ClockField::Second if matches!(lead, '0'..='5') => {
            u8::try_from(lead.to_digit(10)?).ok()?
        }
        ClockField::Hour | ClockField::Minute | ClockField::Second => return None,
    };
    tens.checked_mul(10)?.checked_add(decimal_value(tail)?)
}

fn decimal_value(character: char) -> Option<u8> {
    let code = u32::from(character);
    DECIMAL_ZEROS.chars().find_map(|zero| {
        let offset = code.checked_sub(u32::from(zero))?;
        if offset < 10 {
            u8::try_from(offset).ok()
        } else {
            None
        }
    })
}
