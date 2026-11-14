use std::path::Path;

use num_traits::ToPrimitive as _;
use rust_decimal::Decimal;
use unicode_general_category::{GeneralCategory, get_general_category};
use unicode_width::UnicodeWidthChar as _;

use super::session::Session;
use super::transcript::model_label;

const BAR_WIDTH: usize = 24;
const BAR_FULL: &str = "\u{2588}";
const BAR_EMPTY: &str = "\u{2591}";
const SEPARATOR: &str = " \u{b7} ";
const UNAVAILABLE: &str = "Savings unavailable";
const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[90m";
const ROUTER_COLOR: &str = "\x1b[38;2;79;70;229m";
const BASELINE_COLOR: &str = "\x1b[38;2;217;119;87m";
struct Comparison<'model> {
    spend: Decimal,
    baseline: Decimal,
    baseline_model: &'model str,
    coverage: String,
}
enum Verdict<'model> {
    Hidden,
    Unavailable,
    Compared(Comparison<'model>),
}

// The model the user typed, or its display name from the gateway's model cache, with a model's bare id prettified
pub(crate) fn baseline_label(model: &str, config_dir: &Path) -> String {
    let labelled = model_label(model, config_dir);
    let bare = model.rsplit_once('/').map_or(model, |(_, tail)| tail);
    if labelled != bare {
        return labelled;
    }
    labelled
        .replace('-', " ")
        .split_whitespace()
        .map(|word| {
            let mut letters = word.chars();
            letters.next().map_or_else(String::new, |first| {
                first
                    .to_uppercase()
                    .chain(letters.flat_map(char::to_lowercase))
                    .collect()
            })
        })
        .collect::<Vec<_>>()
        .join(" ")
}
// Combining marks take no column and East Asian wide characters take two
fn display_width(label: &str) -> usize {
    label
        .chars()
        .filter(|character| {
            !matches!(
                get_general_category(*character),
                GeneralCategory::NonspacingMark | GeneralCategory::EnclosingMark
            )
        })
        .map(|character| {
            if character.width() == Some(2) {
                2_usize
            } else {
                1_usize
            }
        })
        .sum()
}
fn paint(use_color: bool, code: &str, text: &str) -> String {
    if use_color {
        format!("{code}{text}{RESET}")
    } else {
        text.to_owned()
    }
}
fn bar(fraction: Decimal, color: &str, use_color: bool) -> Option<String> {
    let filled = fraction
        .clamp(Decimal::ZERO, Decimal::ONE)
        .checked_mul(Decimal::from(BAR_WIDTH))?
        .round()
        .to_usize()?;
    let full = BAR_FULL.repeat(filled);
    let empty = BAR_EMPTY.repeat(BAR_WIDTH.saturating_sub(filled));
    Some(if use_color {
        format!("{color}{full}{DIM}{empty}{RESET}")
    } else {
        format!("{full}{empty}")
    })
}
// Which of LiteLLM's branches applies: nothing to compare, savings that cannot be told, or a comparison to draw
fn verdict(session: &Session) -> Verdict<'_> {
    let Some(baseline) = session.baseline_spend else {
        return Verdict::Unavailable;
    };
    if session.savings_estimated_turns == Some(0) {
        return Verdict::Unavailable;
    }
    let Some(baseline_model) = session
        .baseline_model
        .as_deref()
        .filter(|_| baseline > Decimal::ZERO)
    else {
        return Verdict::Hidden;
    };
    let Some(estimated) = session.savings_estimated_turns else {
        return Verdict::Compared(Comparison {
            spend: session.spend,
            baseline,
            baseline_model,
            coverage: String::new(),
        });
    };
    match (session.savings_estimated_actual_spend, session.turns) {
        (Some(actual), Some(turns)) if estimated <= turns => Verdict::Compared(Comparison {
            spend: actual,
            baseline,
            baseline_model,
            coverage: format!("{SEPARATOR}{estimated} of {turns} turns estimated"),
        }),
        _ => Verdict::Unavailable,
    }
}
struct Chart {
    peak: Decimal,
    label_width: usize,
    use_color: bool,
}

impl Chart {
    fn row(&self, label: &str, amount: Decimal, color: &str) -> Option<String> {
        let padding = " ".repeat(self.label_width.saturating_sub(display_width(label)));
        let drawn = bar(amount.checked_div(self.peak)?, color, self.use_color)?;
        let money = paint(self.use_color, DIM, &format!("${:.2}", amount.round_dp(2)));
        Some(format!(
            "{} {drawn} {money}",
            paint(self.use_color, DIM, &format!("{label}{padding}"))
        ))
    }
}

// None when the money is too large for the arithmetic, so the caller reports savings that cannot be told
fn comparison_lines(
    routed: &str,
    session: &Session,
    compared: &Comparison<'_>,
    config_dir: &Path,
    use_color: bool,
) -> Option<String> {
    let reference = baseline_label(compared.baseline_model, config_dir);
    let percent = compared
        .baseline
        .checked_sub(compared.spend)?
        .checked_div(compared.baseline)?
        .checked_mul(Decimal::ONE_HUNDRED)?
        .round()
        .to_i64()?;
    let sign = match percent {
        1.. => "-",
        0 => "",
        ..=-1 => "+",
    };
    let delta = paint(
        use_color,
        ROUTER_COLOR,
        &format!("{sign}{}% vs {reference}", percent.unsigned_abs()),
    );
    let chart = Chart {
        peak: compared.spend.max(compared.baseline),
        label_width: display_width(&session.router_name).max(display_width(&reference)),
        use_color,
    };
    let router_row = chart.row(&session.router_name, compared.spend, ROUTER_COLOR)?;
    let baseline_row = chart.row(&reference, compared.baseline, BASELINE_COLOR)?;
    Some(format!(
        "{routed}  {delta}{}\n{router_row}\n{baseline_row}",
        compared.coverage
    ))
}

pub(crate) fn render(
    model: &str,
    session: Option<&Session>,
    config_dir: &Path,
    use_color: bool,
) -> String {
    let routed = paint(use_color, BOLD, &format!("Routed to: {model}"));
    let Some(found) = session else {
        return routed;
    };
    match verdict(found) {
        Verdict::Hidden => routed,
        Verdict::Unavailable => format!("{routed}{SEPARATOR}{UNAVAILABLE}"),
        Verdict::Compared(compared) => {
            comparison_lines(&routed, found, &compared, config_dir, use_color)
                .unwrap_or_else(|| format!("{routed}{SEPARATOR}{UNAVAILABLE}"))
        }
    }
}

mod tests;
