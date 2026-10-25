use std::collections::BTreeSet;
use std::fmt;

use serde::de::DeserializeOwned;

use crate::loader::{ConfigError, EnvironmentLookup, parse_view};

pub trait Section: DeserializeOwned + Default {
    type Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup;
}

macro_rules! stub_section {
    () => {
        #[derive(Debug, Default, serde::Deserialize)]
        pub struct Raw {}

        #[derive(Debug, Clone, Default)]
        pub struct Settings;

        impl crate::sections::Section for Raw {
            type Settings = Settings;

            fn convert<L>(self, _lookup: &L) -> Result<Settings, crate::loader::ConfigError>
            where
                L: crate::loader::EnvironmentLookup,
            {
                Ok(Settings)
            }
        }
    };
}
pub(crate) use stub_section;

macro_rules! sections {
    ($($name:ident),* $(,)?) => {
        $(pub mod $name;)*

        #[derive(Clone, Default)]
        pub struct Sections {
            $(pub $name: $name::Settings,)*
        }

        // each section snapshots its own settings, so a new section never changes a shared snapshot
        impl fmt::Debug for Sections {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("Sections")
            }
        }

        pub(crate) struct RawSections {
            $($name: $name::Raw,)*
        }

        impl RawSections {
            pub(crate) fn parse(
                source: &str,
                ignored: &mut Vec<BTreeSet<String>>,
            ) -> Result<Self, ConfigError> {
                Ok(Self {
                    $($name: {
                        let (raw, paths) = parse_view(source)?;
                        ignored.push(paths);
                        raw
                    },)*
                })
            }

            pub(crate) fn convert<L>(self, lookup: &L) -> Result<Sections, ConfigError>
            where
                L: EnvironmentLookup,
            {
                Ok(Sections {
                    $($name: Section::convert(self.$name, lookup)?,)*
                })
            }
        }
    };
}

sections! {
    probe_section,
}
