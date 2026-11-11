use std::collections::BTreeMap;
use std::io::{self, BufReader, ErrorKind, Read};

use rust_decimal::Decimal;

const ID3V1_BYTES: u64 = 128;
const ID3V2_FOOTER_BYTES: u64 = 10;
const APE_FOOTER_BYTES: u64 = 16;
const XING_FIELDS_BYTES: usize = 12;
const FRAME_HEADER_BYTES: u64 = 4;
// the frame header and the Xing fields are read before the rest of the frame is skipped
const FRAME_READ_BYTES: u64 = 16;

// kilobits per second by MPEG version (1, 2, 2.5), layer (I, II, III) and the header's 4-bit index
const BIT_RATES: [[[u64; 15]; 3]; 3] = [
    [
        [
            0, 32, 64, 96, 128, 160, 192, 224, 256, 288, 320, 352, 384, 416, 448,
        ],
        [
            0, 32, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 384,
        ],
        [
            0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320,
        ],
    ],
    [
        [
            0, 32, 48, 56, 64, 80, 96, 112, 128, 144, 160, 176, 192, 224, 256,
        ],
        [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160],
        [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160],
    ],
    [
        [
            0, 32, 48, 56, 64, 80, 96, 112, 128, 144, 160, 176, 192, 224, 256,
        ],
        [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160],
        [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160],
    ],
];
const SAMPLE_RATES: [[u32; 3]; 3] = [
    [44100, 48000, 32000],
    [22050, 24000, 16000],
    [11025, 12000, 8000],
];
const SAMPLES_PER_FRAME: [[u32; 3]; 3] = [[384, 1152, 1152], [384, 1152, 576], [384, 1152, 576]];

struct Frame {
    sample_rate: u32,
    samples: u32,
    length: u64,
    side_information: u64,
}

enum Next {
    End,
    Header([u8; 4]),
}

enum Counted {
    Declared(Decimal),
    Frame { sample_rate: u32, samples: u32 },
}

pub(super) fn mp3_seconds<R>(source: R) -> Option<Decimal>
where
    R: Read,
{
    let mut media = BufReader::new(source);
    let mut samples_by_rate: BTreeMap<u32, u64> = BTreeMap::new();
    while let Next::Header(bytes) = next_header(&mut media)? {
        if bytes.first() != Some(&0xFF) || bytes.get(1).is_none_or(|byte| byte & 0xE0 != 0xE0) {
            skip_tag(&mut media, bytes)?;
            continue;
        }
        match read_frame(&mut media, bytes)? {
            Counted::Declared(total) => return Some(total),
            Counted::Frame {
                sample_rate,
                samples,
            } => {
                let total = samples_by_rate.entry(sample_rate).or_default();
                *total = total.saturating_add(u64::from(samples));
            }
        }
    }
    seconds(&samples_by_rate)
}

fn next_header<R>(media: &mut R) -> Option<Next>
where
    R: Read,
{
    let mut bytes = [0_u8; 4];
    while bytes.first() == Some(&0) {
        match media.read_exact(bytes.get_mut(..1)?) {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::UnexpectedEof => return Some(Next::End),
            Err(_) => return None,
        }
    }
    match media.read_exact(bytes.get_mut(1..)?) {
        Ok(()) => Some(Next::Header(bytes)),
        Err(error) if error.kind() == ErrorKind::UnexpectedEof => Some(Next::End),
        Err(_) => None,
    }
}

fn read_frame<R>(media: &mut R, header: [u8; 4]) -> Option<Counted>
where
    R: Read,
{
    let frame = parse_frame(u32::from_be_bytes(header))?;
    skip(media, frame.side_information)?;
    let mut fields = [0_u8; XING_FIELDS_BYTES];
    media.read_exact(&mut fields).ok()?;
    if let Some(frames) = xing_frames(&fields) {
        let samples = u64::from(frames).checked_mul(u64::from(frame.samples))?;
        return Some(Counted::Declared(seconds(&BTreeMap::from([(
            frame.sample_rate,
            samples,
        )]))?));
    }
    let read = frame.side_information.checked_add(FRAME_READ_BYTES)?;
    skip(media, frame.length.checked_sub(read)?)?;
    Some(Counted::Frame {
        sample_rate: frame.sample_rate,
        samples: frame.samples,
    })
}

fn parse_frame(header: u32) -> Option<Frame> {
    let version = match (header >> 19_u32) & 0b11_u32 {
        0 => 2,
        2 => 1,
        3 => 0,
        _ => return None,
    };
    let layer = match (header >> 17_u32) & 0b11_u32 {
        1 => 2,
        2 => 1,
        3 => 0,
        _ => return None,
    };
    let bitrate_index = usize::try_from((header >> 12_u32) & 0b1111_u32).ok()?;
    let rate_index = usize::try_from((header >> 10_u32) & 0b11_u32).ok()?;
    let kilobits = *BIT_RATES.get(version)?.get(layer)?.get(bitrate_index)?;
    let sample_rate = *SAMPLE_RATES.get(version)?.get(rate_index)?;
    let samples = *SAMPLES_PER_FRAME.get(version)?.get(layer)?;
    if kilobits == 0 {
        return None;
    }
    let bytes_per_second =
        u64::from(samples.checked_div(8)?).checked_mul(kilobits.checked_mul(1000)?)?;
    let padding = u64::from((header >> 9_u32) & 1_u32);
    let mono = (header >> 6_u32) & 0b11_u32 == 3;
    Some(Frame {
        sample_rate,
        samples,
        length: bytes_per_second
            .checked_div(u64::from(sample_rate))?
            .checked_add(padding)?,
        side_information: match (version, mono) {
            (0, true) => 17,
            (0, false) => 32,
            (_, true) => 9,
            (_, false) => 17,
        },
    })
}

fn xing_frames(fields: &[u8; XING_FIELDS_BYTES]) -> Option<u32> {
    let tag = fields.get(..4)?;
    if (tag != b"Xing" && tag != b"Info") || (fields.get(7)? & 1) == 0 {
        return None;
    }
    Some(u32::from_be_bytes(fields.get(8..12)?.try_into().ok()?))
}

fn skip_tag<R>(media: &mut R, bytes: [u8; 4]) -> Option<()>
where
    R: Read,
{
    match &bytes {
        [b'I', b'D', b'3', _] => {
            let mut rest = [0_u8; 6];
            media.read_exact(&mut rest).ok()?;
            let size = rest
                .get(2..6)?
                .iter()
                .fold(0_u64, |size, byte| (size << 7_u32) | u64::from(*byte));
            let footer = if rest.get(1)? & 0b0001_0000 == 0 {
                0
            } else {
                ID3V2_FOOTER_BYTES
            };
            skip(media, size.checked_add(footer)?)
        }
        [b'T', b'A', b'G', _] => skip(media, ID3V1_BYTES - FRAME_HEADER_BYTES),
        b"APET" => {
            let mut rest = [0_u8; 12];
            media.read_exact(&mut rest).ok()?;
            if rest.get(..4)? != b"AGEX" {
                return None;
            }
            let size = u32::from_le_bytes(rest.get(8..12)?.try_into().ok()?);
            skip(media, u64::from(size).checked_add(APE_FOOTER_BYTES)?)
        }
        _ => None,
    }
}

fn skip<R>(media: &mut R, count: u64) -> Option<()>
where
    R: Read,
{
    let mut window = media.by_ref().take(count);
    let copied = io::copy(&mut window, &mut io::sink()).ok()?;
    (copied == count).then_some(())
}

fn seconds(samples_by_rate: &BTreeMap<u32, u64>) -> Option<Decimal> {
    let mut total = Decimal::ZERO;
    for (rate, samples) in samples_by_rate {
        total = total.checked_add(Decimal::from(*samples).checked_div(Decimal::from(*rate))?)?;
    }
    (!total.is_zero()).then_some(total)
}
