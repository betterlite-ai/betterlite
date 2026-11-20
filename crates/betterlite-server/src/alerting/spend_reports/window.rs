use std::error::Error as StdError;
use std::fmt::Write as _;

use betterlite_logging::analytics::python::float_text;
use chrono::{Days, Months, NaiveDate};

use super::{SPEND_REPORT_PLACES, SpendReportError, float_rounded};
use crate::spend::estimates::report::SpendRow;

const MONTHLY_TTL_DAYS: u64 = 30;
const SECONDS_IN_AN_HOUR: u64 = 3_600;
const SECONDS_IN_A_DAY: u64 = 86_400;

#[derive(Clone, Copy)]
pub(super) enum ReportKind {
    Weekly { days: u32 },
    Monthly,
}

// LiteLLM builds the window, the marker, the message and the TTL twice; one type keeps both reports on the same code
#[derive(Clone, Copy)]
pub(super) struct ReportWindow {
    kind: ReportKind,
    pub(super) first: NaiveDate,
    pub(super) last: NaiveDate,
}

impl ReportWindow {
    pub(super) fn new<E>(kind: ReportKind, today: NaiveDate) -> Result<Self, SpendReportError<E>>
    where
        E: StdError + 'static,
    {
        let (first, last) = match kind {
            ReportKind::Weekly { days } => (
                today
                    .checked_sub_days(Days::new(u64::from(days)))
                    .ok_or(SpendReportError::Window { days })?,
                today,
            ),
            ReportKind::Monthly => {
                let first = today.with_day(1).ok_or(SpendReportError::MonthWindow)?;
                let next_month = first
                    .checked_add_months(Months::new(1))
                    .ok_or(SpendReportError::MonthWindow)?;
                (
                    first,
                    next_month.pred_opt().ok_or(SpendReportError::MonthWindow)?,
                )
            }
        };
        Ok(Self { kind, first, last })
    }

    pub(super) fn marker(self, first_text: &str, last_text: &str) -> String {
        let name = match self.kind {
            ReportKind::Weekly { .. } => "weekly",
            ReportKind::Monthly => "monthly",
        };
        format!("{name}_spend_report_sent_{first_text}_{last_text}")
    }

    pub(super) fn message(self, teams: &[SpendRow], tags: &[SpendRow]) -> String {
        let mut message = match self.kind {
            ReportKind::Weekly { days } => format!(
                "*💸 Spend Report for `{} - {}` ({days} days)*\n",
                self.first.format("%m-%d-%Y"),
                self.last.format("%m-%d-%Y"),
            ),
            ReportKind::Monthly => format!(
                "*💸 Monthly Spend Report for `{} - {}` *\n",
                self.first.format("%m-%d-%Y"),
                self.last.format("%m-%d-%Y"),
            ),
        };
        push_sections(&mut message, teams, tags);
        message
    }

    pub(super) fn ttl_seconds(self, hours_in_a_day: u32) -> u64 {
        let day_ttl = u64::from(hours_in_a_day).saturating_mul(SECONDS_IN_AN_HOUR);
        match self.kind {
            ReportKind::Weekly { days } => u64::from(days).saturating_mul(SECONDS_IN_A_DAY),
            ReportKind::Monthly => day_ttl.saturating_mul(MONTHLY_TTL_DAYS),
        }
    }
}

// LiteLLM writes both headings even for an empty list, and a missing alias as None
fn push_sections(message: &mut String, teams: &[SpendRow], tags: &[SpendRow]) {
    push_rows(message, "Team", teams);
    push_rows(message, "Tag", tags);
}

fn push_rows(message: &mut String, label: &str, rows: &[SpendRow]) {
    let _ignored = writeln!(message, "\n*{label} Spend Report:*");
    for row in rows {
        let spend = float_text(float_rounded(row.total_spend, SPEND_REPORT_PLACES).as_f64());
        let name = row.name.as_deref().unwrap_or("None");
        let _ignored = writeln!(message, "{label}: `{name}` | Spend: `${spend}`");
    }
}
