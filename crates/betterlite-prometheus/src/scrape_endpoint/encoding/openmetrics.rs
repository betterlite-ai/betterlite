use std::fmt::Write as _;

use crate::collector_registry::CollectorRegistry;

use super::{escape_help, families, sample_line};

// openmetrics.generate_latest: the registered family names, every sample inline where collect yields it, then the EOF marker
pub(crate) fn render_openmetrics(registry: &CollectorRegistry, metric_names: &[String]) -> Vec<u8> {
    let mut output = String::new();
    for family in families(registry, metric_names) {
        let help = escape_help(&family.help, true);
        let _help_line = writeln!(output, "# HELP {} {help}", family.name);
        let _type_line = writeln!(output, "# TYPE {} {}", family.name, family.kind.as_str());
        for sample in &family.samples {
            output.push_str(&sample_line(sample));
        }
    }
    output.push_str("# EOF\n");
    output.into_bytes()
}
