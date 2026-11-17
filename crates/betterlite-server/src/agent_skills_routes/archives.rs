use betterlite_http::skill_archive::{SkillArchive, build_skill_archive};

// Repacking an upload is blocking CPU work, so a caller runs this port on the blocking pool
pub trait ArchiveBuilder: Send + Sync {
    fn build(&self, upload: &[u8]) -> Option<SkillArchive>;
}

pub struct ZipArchiveBuilder;

impl ArchiveBuilder for ZipArchiveBuilder {
    fn build(&self, upload: &[u8]) -> Option<SkillArchive> {
        build_skill_archive(upload)
    }
}
