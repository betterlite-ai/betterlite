use chrono::{DateTime, Utc};
use jiff::civil;
use jiff::tz::{AmbiguousOffset, TimeZone, TimeZoneDatabase};

use super::error::{DurationError, DurationWarning};
use super::reset::Fold;

pub(crate) struct ResolvedZone {
    pub(crate) zone: TimeZone,
    pub(crate) warnings: Vec<DurationWarning>,
}

pub(crate) fn resolve_zone(name: Option<&str>, database: &TimeZoneDatabase) -> ResolvedZone {
    let Some(requested) = name else {
        return ResolvedZone {
            zone: TimeZone::UTC,
            warnings: Vec::new(),
        };
    };
    // Python's ZoneInfo matches the on-disk name case-sensitively, while jiff's lookup ignores ASCII case
    let resolved = database
        .get(requested)
        .ok()
        .filter(|zone| zone.iana_name() == Some(requested));
    resolved.map_or_else(
        || ResolvedZone {
            zone: TimeZone::UTC,
            warnings: vec![DurationWarning::InvalidTimezone {
                name: requested.to_owned(),
            }],
        },
        |zone| ResolvedZone {
            zone,
            warnings: Vec::new(),
        },
    )
}

pub(crate) fn civil_time(
    moment: DateTime<Utc>,
    zone: &TimeZone,
) -> Result<(civil::DateTime, Fold), DurationError> {
    let timestamp = timestamp_of(moment)?;
    let civil = zone.to_datetime(timestamp);
    Ok((civil, fold_of(zone, civil, timestamp)?))
}

pub(crate) fn instant(
    civil: civil::DateTime,
    zone: &TimeZone,
    fold: Fold,
) -> Result<DateTime<Utc>, DurationError> {
    let ambiguous = zone.to_ambiguous_timestamp(civil);
    let offset = match (ambiguous.offset(), fold) {
        (AmbiguousOffset::Unambiguous { offset }, _) => offset,
        (
            AmbiguousOffset::Gap { before, .. } | AmbiguousOffset::Fold { before, .. },
            Fold::Before,
        ) => before,
        (AmbiguousOffset::Gap { after, .. } | AmbiguousOffset::Fold { after, .. }, Fold::After) => {
            after
        }
    };
    let timestamp = offset
        .to_timestamp(civil)
        .map_err(|error| DurationError::Timestamp { source: error })?;
    chrono_time(timestamp)
}

fn timestamp_of(moment: DateTime<Utc>) -> Result<jiff::Timestamp, DurationError> {
    let nanos = i32::try_from(moment.timestamp_subsec_nanos())
        .map_err(|error| DurationError::CalendarField { source: error })?;
    jiff::Timestamp::new(moment.timestamp(), nanos)
        .map_err(|error| DurationError::Timestamp { source: error })
}

fn chrono_time(timestamp: jiff::Timestamp) -> Result<DateTime<Utc>, DurationError> {
    let nanos = u32::try_from(timestamp.subsec_nanosecond())
        .map_err(|error| DurationError::CalendarField { source: error })?;
    DateTime::from_timestamp(timestamp.as_second(), nanos).ok_or(DurationError::TimestampOutOfRange)
}

fn fold_of(
    zone: &TimeZone,
    civil: civil::DateTime,
    timestamp: jiff::Timestamp,
) -> Result<Fold, DurationError> {
    let ambiguous = zone.to_ambiguous_timestamp(civil);
    match ambiguous.offset() {
        // A real instant always falls on a civil time the zone holds, so a gap is unreachable here.
        AmbiguousOffset::Unambiguous { .. } | AmbiguousOffset::Gap { .. } => Ok(Fold::Before),
        AmbiguousOffset::Fold { .. } => {
            let earlier = ambiguous
                .earlier()
                .map_err(|error| DurationError::Timestamp { source: error })?;
            Ok(if earlier == timestamp {
                Fold::Before
            } else {
                Fold::After
            })
        }
    }
}
