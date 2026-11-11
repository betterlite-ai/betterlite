use std::io::{Cursor, Read, Seek, SeekFrom};

use lewton::header::read_header_ident;
use rust_decimal::Decimal;

use super::mp3::mp3_seconds;

const OGG_HEADER_BYTES: usize = 27;
const OGG_MAX_SEGMENTS: usize = 255;
// the three Vorbis headers fit far below this, so an oversized first packet is refused unread
const OGG_HEADER_PROBE_BYTES: u64 = 0x0002_0000;

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
            [b'I', b'D', b'3', ..] | [0xFF, ..] => Some(Self::Mp3),
            _ => None,
        }
    }
}

pub(super) fn container_seconds<R>(container: Container, mut media: R) -> Option<Decimal>
where
    R: Read + Seek,
{
    match container {
        Container::Wav => {
            let reader = hound::WavReader::new(media).ok()?;
            exact_seconds(u64::from(reader.duration()), reader.spec().sample_rate)
        }
        Container::Flac => {
            let options = claxon::FlacReaderOptions {
                metadata_only: true,
                read_vorbis_comment: false,
            };
            let stream = claxon::FlacReader::new_ext(media, options)
                .ok()?
                .streaminfo();
            exact_seconds(stream.samples?, stream.sample_rate)
        }
        Container::Ogg => {
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
            exact_seconds(last_ogg_granule(&mut media)?, ident.audio_sample_rate)
        }
        Container::Mp3 => mp3_seconds(media),
    }
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
