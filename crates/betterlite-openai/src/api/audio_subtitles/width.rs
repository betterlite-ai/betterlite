const CJK_RANGES: [(char, char); 6] = [
    ('\u{3400}', '\u{4DBF}'),
    ('\u{4E00}', '\u{9FFF}'),
    ('\u{F900}', '\u{FAFF}'),
    ('\u{3040}', '\u{309F}'),
    ('\u{30A0}', '\u{30FF}'),
    ('\u{31F0}', '\u{31FF}'),
];

const CJK_NO_BREAK_BEFORE: &str = "\u{3001}\u{3002}\u{ff0c}\u{ff0e}\u{ff01}\u{ff1f}\u{ff1a}\u{ff1b}\u{30fb}\u{30fc}\u{2026}\u{300d}\u{300f}\u{ff09}\u{3009}\u{300b}\u{3011}\u{3015}";

const CJK_NO_BREAK_AFTER: &str = "\u{300c}\u{300e}\u{ff08}\u{3008}\u{300a}\u{3010}\u{3014}";

// Python's unicodedata.east_asian_width reports W or F for exactly these Unicode 15.1 code points
const EAST_ASIAN_WIDE: [(char, char); 120] = [
    ('\u{1100}', '\u{115F}'),
    ('\u{231A}', '\u{231B}'),
    ('\u{2329}', '\u{232A}'),
    ('\u{23E9}', '\u{23EC}'),
    ('\u{23F0}', '\u{23F0}'),
    ('\u{23F3}', '\u{23F3}'),
    ('\u{25FD}', '\u{25FE}'),
    ('\u{2614}', '\u{2615}'),
    ('\u{2648}', '\u{2653}'),
    ('\u{267F}', '\u{267F}'),
    ('\u{2693}', '\u{2693}'),
    ('\u{26A1}', '\u{26A1}'),
    ('\u{26AA}', '\u{26AB}'),
    ('\u{26BD}', '\u{26BE}'),
    ('\u{26C4}', '\u{26C5}'),
    ('\u{26CE}', '\u{26CE}'),
    ('\u{26D4}', '\u{26D4}'),
    ('\u{26EA}', '\u{26EA}'),
    ('\u{26F2}', '\u{26F3}'),
    ('\u{26F5}', '\u{26F5}'),
    ('\u{26FA}', '\u{26FA}'),
    ('\u{26FD}', '\u{26FD}'),
    ('\u{2705}', '\u{2705}'),
    ('\u{270A}', '\u{270B}'),
    ('\u{2728}', '\u{2728}'),
    ('\u{274C}', '\u{274C}'),
    ('\u{274E}', '\u{274E}'),
    ('\u{2753}', '\u{2755}'),
    ('\u{2757}', '\u{2757}'),
    ('\u{2795}', '\u{2797}'),
    ('\u{27B0}', '\u{27B0}'),
    ('\u{27BF}', '\u{27BF}'),
    ('\u{2B1B}', '\u{2B1C}'),
    ('\u{2B50}', '\u{2B50}'),
    ('\u{2B55}', '\u{2B55}'),
    ('\u{2E80}', '\u{2E99}'),
    ('\u{2E9B}', '\u{2EF3}'),
    ('\u{2F00}', '\u{2FD5}'),
    ('\u{2FF0}', '\u{303E}'),
    ('\u{3041}', '\u{3096}'),
    ('\u{3099}', '\u{30FF}'),
    ('\u{3105}', '\u{312F}'),
    ('\u{3131}', '\u{318E}'),
    ('\u{3190}', '\u{31E3}'),
    ('\u{31EF}', '\u{321E}'),
    ('\u{3220}', '\u{3247}'),
    ('\u{3250}', '\u{4DBF}'),
    ('\u{4E00}', '\u{A48C}'),
    ('\u{A490}', '\u{A4C6}'),
    ('\u{A960}', '\u{A97C}'),
    ('\u{AC00}', '\u{D7A3}'),
    ('\u{F900}', '\u{FAFF}'),
    ('\u{FE10}', '\u{FE19}'),
    ('\u{FE30}', '\u{FE52}'),
    ('\u{FE54}', '\u{FE66}'),
    ('\u{FE68}', '\u{FE6B}'),
    ('\u{FF01}', '\u{FF60}'),
    ('\u{FFE0}', '\u{FFE6}'),
    ('\u{16FE0}', '\u{16FE4}'),
    ('\u{16FF0}', '\u{16FF1}'),
    ('\u{17000}', '\u{187F7}'),
    ('\u{18800}', '\u{18CD5}'),
    ('\u{18D00}', '\u{18D08}'),
    ('\u{1AFF0}', '\u{1AFF3}'),
    ('\u{1AFF5}', '\u{1AFFB}'),
    ('\u{1AFFD}', '\u{1AFFE}'),
    ('\u{1B000}', '\u{1B122}'),
    ('\u{1B132}', '\u{1B132}'),
    ('\u{1B150}', '\u{1B152}'),
    ('\u{1B155}', '\u{1B155}'),
    ('\u{1B164}', '\u{1B167}'),
    ('\u{1B170}', '\u{1B2FB}'),
    ('\u{1F004}', '\u{1F004}'),
    ('\u{1F0CF}', '\u{1F0CF}'),
    ('\u{1F18E}', '\u{1F18E}'),
    ('\u{1F191}', '\u{1F19A}'),
    ('\u{1F200}', '\u{1F202}'),
    ('\u{1F210}', '\u{1F23B}'),
    ('\u{1F240}', '\u{1F248}'),
    ('\u{1F250}', '\u{1F251}'),
    ('\u{1F260}', '\u{1F265}'),
    ('\u{1F300}', '\u{1F320}'),
    ('\u{1F32D}', '\u{1F335}'),
    ('\u{1F337}', '\u{1F37C}'),
    ('\u{1F37E}', '\u{1F393}'),
    ('\u{1F3A0}', '\u{1F3CA}'),
    ('\u{1F3CF}', '\u{1F3D3}'),
    ('\u{1F3E0}', '\u{1F3F0}'),
    ('\u{1F3F4}', '\u{1F3F4}'),
    ('\u{1F3F8}', '\u{1F43E}'),
    ('\u{1F440}', '\u{1F440}'),
    ('\u{1F442}', '\u{1F4FC}'),
    ('\u{1F4FF}', '\u{1F53D}'),
    ('\u{1F54B}', '\u{1F54E}'),
    ('\u{1F550}', '\u{1F567}'),
    ('\u{1F57A}', '\u{1F57A}'),
    ('\u{1F595}', '\u{1F596}'),
    ('\u{1F5A4}', '\u{1F5A4}'),
    ('\u{1F5FB}', '\u{1F64F}'),
    ('\u{1F680}', '\u{1F6C5}'),
    ('\u{1F6CC}', '\u{1F6CC}'),
    ('\u{1F6D0}', '\u{1F6D2}'),
    ('\u{1F6D5}', '\u{1F6D7}'),
    ('\u{1F6DC}', '\u{1F6DF}'),
    ('\u{1F6EB}', '\u{1F6EC}'),
    ('\u{1F6F4}', '\u{1F6FC}'),
    ('\u{1F7E0}', '\u{1F7EB}'),
    ('\u{1F7F0}', '\u{1F7F0}'),
    ('\u{1F90C}', '\u{1F93A}'),
    ('\u{1F93C}', '\u{1F945}'),
    ('\u{1F947}', '\u{1F9FF}'),
    ('\u{1FA70}', '\u{1FA7C}'),
    ('\u{1FA80}', '\u{1FA88}'),
    ('\u{1FA90}', '\u{1FABD}'),
    ('\u{1FABF}', '\u{1FAC5}'),
    ('\u{1FACE}', '\u{1FADB}'),
    ('\u{1FAE0}', '\u{1FAE8}'),
    ('\u{1FAF0}', '\u{1FAF8}'),
    ('\u{20000}', '\u{2FFFD}'),
    ('\u{30000}', '\u{3FFFD}'),
];

#[must_use]
pub fn is_cjk(ch: char) -> bool {
    CJK_RANGES
        .iter()
        .any(|&(low, high)| (low..=high).contains(&ch))
}

#[must_use]
pub fn is_cjk_word_boundary(prev: char, next: char) -> bool {
    if !is_cjk(prev) && !is_cjk(next) {
        return false;
    }
    !CJK_NO_BREAK_BEFORE.contains(next) && !CJK_NO_BREAK_AFTER.contains(prev)
}

#[must_use]
pub fn text_width(text: &str) -> usize {
    text.chars()
        .map(|ch| if is_wide_or_fullwidth(ch) { 2 } else { 1 })
        .sum()
}

pub(crate) const fn is_text_space(ch: char) -> bool {
    if matches!(ch, '\u{1c}'..='\u{1f}') {
        return true;
    }
    ch.is_whitespace()
}

fn is_wide_or_fullwidth(ch: char) -> bool {
    EAST_ASIAN_WIDE
        .iter()
        .any(|&(low, high)| (low..=high).contains(&ch))
}
