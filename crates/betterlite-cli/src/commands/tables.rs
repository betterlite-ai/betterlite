use std::io::Write;

use comfy_table::Table;
use serde_json::value::RawValue;

#[derive(Clone, Copy, Debug, Eq, PartialEq, clap::ValueEnum)]
pub(super) enum OutputFormat {
    Table,
    Json,
}

pub(super) fn print_http_error(
    status: u16,
    body: &[u8],
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) {
    let _wrote_status = writeln!(stderr, "Error: HTTP {status}");
    let parsed_as_json = serde_json::from_slice::<&RawValue>(body).is_ok();
    let target: &mut dyn Write = if parsed_as_json { stdout } else { stderr };
    let _wrote_body = target.write_all(body);
    if !body.ends_with(b"\n") {
        let _wrote_terminator = target.write_all(b"\n");
    }
}

pub(super) fn write_json_response(
    body: &[u8],
    stdout: &mut dyn Write,
) -> Result<(), serde_json::Error> {
    serde_json::from_slice::<&RawValue>(body)?;
    let _wrote_body = stdout.write_all(body);
    if !body.ends_with(b"\n") {
        let _wrote_terminator = stdout.write_all(b"\n");
    }
    Ok(())
}

pub(super) fn write_table(
    title: &str,
    headers: &[&str],
    rows: Vec<Vec<String>>,
    stdout: &mut dyn Write,
) {
    let mut table = Table::new();
    table.set_header(headers.to_vec());
    for row in rows {
        table.add_row(row);
    }
    // rich draws a table's title on the line above it, and comfy-table has no title of its own
    let _wrote_title = writeln!(stdout, "{title}");
    let _wrote_table = writeln!(stdout, "{table}");
}
