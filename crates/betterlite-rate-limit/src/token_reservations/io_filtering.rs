use crate::limiter_records::{
    PROJECT_ITPM_DESCRIPTOR_KEY, PROJECT_OTPM_DESCRIPTOR_KEY, RateLimitDescriptor,
};

mod tests;

pub(crate) fn split_io_descriptors(
    descriptors: &[RateLimitDescriptor],
) -> (Vec<RateLimitDescriptor>, Vec<RateLimitDescriptor>) {
    let mut itpm = Vec::new();
    let mut otpm = Vec::new();
    for descriptor in descriptors {
        match descriptor.key.as_str() {
            PROJECT_ITPM_DESCRIPTOR_KEY => itpm.push(descriptor.clone()),
            PROJECT_OTPM_DESCRIPTOR_KEY => otpm.push(descriptor.clone()),
            _ => {}
        }
    }
    (itpm, otpm)
}
