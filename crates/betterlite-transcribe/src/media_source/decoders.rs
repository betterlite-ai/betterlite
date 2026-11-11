use std::io::{Cursor, Read, Seek, SeekFrom};
use std::time::Duration;

use lewton::header::read_header_ident;
use rust_decimal::Decimal;

const NANOS_PER_SECOND: u32 = 1_000_000_000;
const OGG_HEADER_BYTES: usize = 27;
const OGG_MAX_SEGMENTS: usize = 255;
// the three Vorbis headers fit far below this, so an oversized first packet is refused unread
const OGG_HEADER_PROBE_BYTES: u64 = 128 * 1024;

#[derive(Clone, Copy, Debug)]
pub(super) enum Container {
    Wav,
    Flac,
    Ogg,
    Mp3,
}

impl Container {
    pub(super) const fn sniff(header: [u8; 12]) -> Option<Self> {
        match header {
            [b'R', b'I', b'F', b'F', .., b'W', b'A', b'V', b'E'] => Some(Self::Wav),
            [b'f', b'L', b'a', b'C', ..] => Some(Self::Flac),
            [b'O', b'g', b'g', b'S', ..] => Some(Self::Ogg),
            [b'I', b'D', b'3', ..] => Some(Self::Mp3),
            [0xFF, second, ..] if second & 0xE0 == 0xE0 => Some(Self::Mp3),
            _ => None,
        }
    }
}

pub(super) fn wav_seconds<R>(media: R) -> Option<Decimal>
where
    R: Read,
{
    let reader = hound::WavReader::new(media).ok()?;
    let spec = reader.spec();
    exact_seconds(u64::from(reader.duration()), spec.sample_rate)
}

pub(super) fn flac_seconds<R>(media: R) -> Option<Decimal>
where
    R: Read,
{
    let options = claxon::FlacReaderOptions {
        metadata_only: true,
        read_vorbis_comment: false,
    };
    let reader = claxon::FlacReader::new_ext(media, options).ok()?;
    let stream = reader.streaminfo();
    exact_seconds(stream.samples?, stream.sample_rate)
}

pub(super) fn ogg_seconds<R>(mut media: R) -> Option<Decimal>
where
    R: Read + Seek,
{
    let sample_rate = ogg_sample_rate(&mut media)?;
    let frames = last_ogg_granule(&mut media)?;
    exact_seconds(frames, sample_rate)
}

pub(super) fn mp3_seconds<R>(mut media: R) -> Option<Decimal>
where
    R: Read,
{
    let Ok(duration) = mp3_duration::from_read(&mut media) else {
        return None;
    };
    duration_seconds(duration)
}

fn ogg_sample_rate<R>(media: &mut R) -> Option<u32>
where
    R: Read + Seek,
{
    media.seek(SeekFrom::Start(0)).ok()?;
    let mut probe = Vec::new();
    media
        .by_ref()
        .take(OGG_HEADER_PROBE_BYTES)
        .read_to_end(&mut probe)
        .ok()?;
    let mut packets = ogg::PacketReader::new(Cursor::new(probe));
    let packet = packets.read_packet().ok()??;
    let ident = read_header_ident(&packet.data).ok()?;
    Some(ident.audio_sample_rate)
}

fn last_ogg_granule<R>(media: &mut R) -> Option<u64>
where
    R: Read + Seek,
{
    media.seek(SeekFrom::Start(0)).ok()?;
    let mut granule = None;
    let mut header = [0_u8; OGG_HEADER_BYTES];
    loop {
        if media.read_exact(&mut header).is_err() {
            break;
        }
        if header.get(..4) != Some(b"OggS".as_slice()) {
            break;
        }
        let page_granule = u64::from_le_bytes(header.get(6..14)?.try_into().ok()?);
        let segment_count = usize::from(*header.get(26)?);
        let mut table = [0_u8; OGG_MAX_SEGMENTS];
        let segments = table.get_mut(..segment_count)?;
        if media.read_exact(segments).is_err() {
            break;
        }
        let body: u64 = segments.iter().map(|length| u64::from(*length)).sum();
        if page_granule != u64::MAX {
            granule = Some(page_granule);
        }
        let Ok(offset) = i64::try_from(body) else {
            break;
        };
        if media.seek(SeekFrom::Current(offset)).is_err() {
            break;
        }
    }
    granule
}

fn exact_seconds(frames: u64, sample_rate: u32) -> Option<Decimal> {
    if sample_rate == 0 {
        return None;
    }
    Decimal::from(frames).checked_div(Decimal::from(sample_rate))
}

fn duration_seconds(duration: Duration) -> Option<Decimal> {
    let whole = Decimal::from(duration.as_secs());
    let fractional =
        Decimal::from(duration.subsec_nanos()).checked_div(Decimal::from(NANOS_PER_SECOND))?;
    whole.checked_add(fractional)
}
