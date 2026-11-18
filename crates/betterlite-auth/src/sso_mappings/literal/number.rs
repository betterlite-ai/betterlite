use super::{Cursor, Literal, LiteralError};

struct Digits {
    count: usize,
    nonzero: bool,
}

pub(super) fn number_start(cursor: &Cursor<'_>) -> bool {
    match cursor.bytes.get(cursor.at).copied() {
        Some(byte) if byte.is_ascii_digit() => true,
        Some(b'.') => {
            let Some(next) = cursor.bytes.get(cursor.at.saturating_add(1)).copied() else {
                return false;
            };
            next.is_ascii_digit()
        }
        _other => false,
    }
}

pub(super) fn parse_number(cursor: &mut Cursor<'_>) -> Result<Literal, LiteralError> {
    let Some(first) = cursor.bytes.get(cursor.at).copied() else {
        return Err(LiteralError::Syntax);
    };
    if first == b'0'
        && let Some(found) = radix(cursor)
    {
        scan_radix(cursor, found)?;
        return Ok(Literal::Number);
    }
    scan_decimal(cursor)?;
    skip_imaginary(cursor);
    Ok(Literal::Number)
}

fn radix(cursor: &Cursor<'_>) -> Option<u32> {
    let lowered = cursor.bytes.get(cursor.at.saturating_add(1))?;
    match lowered {
        b'x' | b'X' => Some(16),
        b'o' | b'O' => Some(8),
        b'b' | b'B' => Some(2),
        _other => None,
    }
}

fn scan_radix(cursor: &mut Cursor<'_>, radix: u32) -> Result<(), LiteralError> {
    cursor.at = cursor.at.saturating_add(2);
    let mut digits = 0_usize;
    let mut after_underscore = false;
    while let Some(byte) = cursor.bytes.get(cursor.at).copied() {
        if byte == b'_' && after_underscore {
            return Err(LiteralError::Syntax);
        }
        if byte == b'_' {
            after_underscore = true;
            cursor.at = cursor.at.saturating_add(1);
            continue;
        }
        if !char::from(byte).is_digit(radix) {
            break;
        }
        digits = digits.saturating_add(1);
        after_underscore = false;
        cursor.at = cursor.at.saturating_add(1);
    }
    if digits == 0 || after_underscore {
        return Err(LiteralError::Syntax);
    }
    Ok(())
}

fn scan_decimal(cursor: &mut Cursor<'_>) -> Result<(), LiteralError> {
    let start = cursor.at;
    let integer = scan_digits(cursor)?;
    let mut is_float = false;
    if cursor.bytes.get(cursor.at).copied() == Some(b'.') {
        is_float = true;
        cursor.at = cursor.at.saturating_add(1);
        let fraction = scan_digits(cursor)?;
        if integer.count == 0 && fraction.count == 0 {
            return Err(LiteralError::Syntax);
        }
    }
    if matches!(cursor.bytes.get(cursor.at).copied(), Some(b'e' | b'E')) {
        is_float = true;
        cursor.at = cursor.at.saturating_add(1);
        if matches!(cursor.bytes.get(cursor.at).copied(), Some(b'+' | b'-')) {
            cursor.at = cursor.at.saturating_add(1);
        }
        let exponent = scan_digits(cursor)?;
        if exponent.count == 0 {
            return Err(LiteralError::Syntax);
        }
    }
    if is_float {
        return Ok(());
    }
    let leading_zero = cursor.bytes.get(start).copied() == Some(b'0');
    if leading_zero && integer.nonzero {
        return Err(LiteralError::Syntax);
    }
    Ok(())
}

fn scan_digits(cursor: &mut Cursor<'_>) -> Result<Digits, LiteralError> {
    let mut digits = Digits {
        count: 0,
        nonzero: false,
    };
    let mut after_underscore = false;
    while let Some(byte) = cursor.bytes.get(cursor.at).copied() {
        if byte.is_ascii_digit() {
            digits.nonzero = digits.nonzero || byte != b'0';
            digits.count = digits.count.saturating_add(1);
            after_underscore = false;
            cursor.at = cursor.at.saturating_add(1);
            continue;
        }
        if byte != b'_' {
            break;
        }
        if after_underscore || digits.count == 0 {
            return Err(LiteralError::Syntax);
        }
        after_underscore = true;
        cursor.at = cursor.at.saturating_add(1);
    }
    if after_underscore {
        return Err(LiteralError::Syntax);
    }
    Ok(digits)
}

fn skip_imaginary(cursor: &mut Cursor<'_>) {
    if matches!(cursor.bytes.get(cursor.at).copied(), Some(b'j' | b'J')) {
        cursor.at = cursor.at.saturating_add(1);
    }
}
