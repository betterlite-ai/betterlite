pub const REDIS_CLUSTER_SLOTS: u16 = 0x4000;

// Redis computes CRC16(key) % 16384, and 16384 is a power of two, so the mask gives the same slot without a remainder that can panic
const SLOT_BITS: u16 = 0x3FFF;

#[must_use]
pub fn keyslot_for_redis_cluster(key: &str) -> u16 {
    let tag = hash_tag(key);
    let crc = crc16::State::<crc16::XMODEM>::calculate(tag.as_bytes());
    crc & SLOT_BITS
}

fn hash_tag(key: &str) -> &str {
    let Some((_prefix, after_open)) = key.split_once('{') else {
        return key;
    };
    let Some((tag, _suffix)) = after_open.split_once('}') else {
        return key;
    };
    if tag.is_empty() {
        return key;
    }
    tag
}

mod tests;
