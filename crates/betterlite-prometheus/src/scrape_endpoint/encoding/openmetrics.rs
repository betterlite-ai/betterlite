use std::fmt::Write as _;

use crate::scrape_endpoint::MetricFamily;

use super::{escape_help, restricted, sample_line};

// openmetrics.generate_latest: the registered family names, every sample inline where collect yields it, then the EOF marker
pub(crate) fn render_openmetrics(families: &[MetricFamily], metric_names: &[String]) -> Vec<u8> {
    let mut output = String::new();
    for selected in restricted(families, metric_names) {
        let family = selected.family;
        let help = escape_help(&family.help, true);
        let _help_line = writeln!(output, "# HELP {} {help}", family.name);
        let _type_line = writeln!(output, "# TYPE {} {}", family.name, family.kind.as_str());
        for sample in selected.samples {
            output.push_str(&sample_line(sample));
        }
    }
    output.push_str("# EOF\n");
    output.into_bytes()
}
