//! CSV import: tokenising, delimiter and header detection, and cell counting.
//!
//! Two entry points exist because the import dialog needs to show the user what
//! it found before committing: [`preview_csv`] reports headers, the detected
//! delimiter, and a few sample rows, while [`parse_csv`] produces the numeric
//! table. Both share one tokeniser so the preview can never disagree with the
//! parse.
//!
//! Cells that cannot be parsed become `NaN` and are counted. They are never
//! dropped, because a silently shortened column would shift every later sample
//! against the time axis.

use serde::{Deserialize, Serialize};

use hex_core::{Real, TimestampUnit};

use crate::error::FlightDataError;

/// Candidate delimiters, in the order used to break a detection tie.
const CANDIDATE_DELIMITERS: [u8; 4] = [b',', b';', b'\t', b'|'];

/// How many rows the preview shows by default.
const PREVIEW_SAMPLE_ROWS: usize = 5;

/// Safety ceiling on the rows scanned solely to report a preview count.
const PREVIEW_SCAN_LIMIT: usize = 200_000;

/// Options controlling CSV tokenising and numeric interpretation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CsvImportOptions {
    /// Field delimiter. Leave at `,` to let the importer detect it.
    pub delimiter: u8,
    /// Whether the caller expects a header row.
    ///
    /// A header is still refused when the first row is unambiguously numeric, so
    /// a numeric first row is never mistaken for column names.
    pub has_header: bool,
    /// Column name to use as the time axis. When absent, the importer looks for
    /// a mapped or name-matched time column.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp_column: Option<String>,
    /// Unit of the raw timestamp values.
    pub timestamp_unit: TimestampUnit,
    /// Lines beginning with this character are skipped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment_prefix: Option<char>,
    /// Hard cap on data rows. Rows beyond the cap are not parsed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_rows: Option<usize>,
    /// Interpret a comma as the decimal separator, as used in much of Europe.
    pub decimal_comma: bool,
}

impl Default for CsvImportOptions {
    fn default() -> Self {
        Self {
            delimiter: b',',
            has_header: true,
            timestamp_column: None,
            timestamp_unit: TimestampUnit::Seconds,
            comment_prefix: None,
            max_rows: None,
            decimal_comma: false,
        }
    }
}

/// What the importer found in a CSV file before any numeric parsing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CsvPreview {
    /// Column names, real or generated.
    pub headers: Vec<String>,
    /// Delimiter that will be used.
    pub delimiter: u8,
    /// Whether the first row was treated as a header.
    pub detected_has_header: bool,
    /// Number of data rows found, capped by `max_rows` when set.
    pub row_count_preview: usize,
    /// The first few data rows, as text.
    pub sample_rows: Vec<Vec<String>>,
    /// Human-readable notes about the decisions taken.
    pub warnings: Vec<String>,
}

/// A numeric table with its source column names.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawTable {
    /// Column names, one per entry in `columns`.
    pub headers: Vec<String>,
    /// One numeric column per header, all the same length.
    pub columns: Vec<Vec<Real>>,
    /// Cells absent from a short row.
    pub missing_cells: usize,
    /// Cells present but not numeric.
    pub unparsable_cells: usize,
    /// Source records consumed, header included, before any row cap applied.
    pub source_lines: usize,
}

impl RawTable {
    /// Number of samples, or zero when the table has no columns.
    pub fn row_count(&self) -> usize {
        self.columns.first().map(|c| c.len()).unwrap_or(0)
    }

    /// Index of the column with the given name, case-insensitively.
    pub fn column_index(&self, name: &str) -> Option<usize> {
        self.headers
            .iter()
            .position(|h| h.eq_ignore_ascii_case(name.trim()))
    }

    /// A column by index.
    pub fn column(&self, index: usize) -> Option<&Vec<Real>> {
        self.columns.get(index)
    }
}

/// Tokenised input shared by the preview and the parser.
struct Tokenised {
    records: Vec<Vec<String>>,
    delimiter: u8,
    delimiter_was_detected: bool,
}

fn delimiter_label(delimiter: u8) -> String {
    match delimiter {
        b'\t' => "tab".to_string(),
        other => (other as char).to_string(),
    }
}

/// Count occurrences of `needle` outside double-quoted spans.
fn count_unquoted(line: &str, needle: char) -> usize {
    let mut count = 0;
    let mut in_quotes = false;
    for ch in line.chars() {
        match ch {
            '"' => in_quotes = !in_quotes,
            c if c == needle && !in_quotes => count += 1,
            _ => {}
        }
    }
    count
}

/// Pick a delimiter from the first non-empty, non-comment line.
///
/// Returns the delimiter and whether it differed from the requested default.
fn detect_delimiter(text: &str, requested: u8, comment_prefix: Option<char>) -> (u8, bool) {
    let first = text
        .lines()
        .find(|line| {
            let trimmed = line.trim();
            !trimmed.is_empty()
                && !comment_prefix
                    .map(|c| trimmed.starts_with(c))
                    .unwrap_or(false)
        })
        .unwrap_or("");

    let requested_char = requested as char;
    let requested_count = count_unquoted(first, requested_char);
    let mut best = (requested, requested_count);
    for candidate in CANDIDATE_DELIMITERS {
        if candidate == requested {
            continue;
        }
        let count = count_unquoted(first, candidate as char);
        if count > best.1 {
            best = (candidate, count);
        }
    }

    // A single-column file has no delimiter at all; keep the requested one.
    if best.1 == 0 {
        return (requested, false);
    }
    (best.0, best.0 != requested)
}

/// True when every non-empty cell of a row fails to parse as a number.
fn row_looks_like_header(cells: &[String], options: &CsvImportOptions) -> bool {
    let mut any = false;
    for cell in cells {
        if cell.trim().is_empty() {
            continue;
        }
        any = true;
        if parse_cell(cell, options).is_some() {
            return false;
        }
    }
    any
}

/// Parse one cell, honouring the decimal-comma convention.
fn parse_cell(raw: &str, options: &CsvImportOptions) -> Option<Real> {
    let text = raw.trim();
    if text.is_empty() {
        return None;
    }
    let normalized = if !options.decimal_comma {
        text.to_string()
    } else {
        let has_comma = text.contains(',');
        let has_dot = text.contains('.');
        match (has_comma, has_dot) {
            // Both present: the dot groups thousands.
            (true, true) => text.replace('.', "").replace(',', "."),
            (true, false) => text.replace(',', "."),
            (false, _) => text.to_string(),
        }
    };
    normalized.parse::<Real>().ok()
}

fn tokenise(text: &str, options: &CsvImportOptions) -> Result<Tokenised, FlightDataError> {
    if text.trim().is_empty() {
        return Err(FlightDataError::EmptyFile);
    }

    let (delimiter, delimiter_was_detected) =
        detect_delimiter(text, options.delimiter, options.comment_prefix);

    let mut builder = csv::ReaderBuilder::new();
    builder
        .delimiter(delimiter)
        .has_headers(false)
        .flexible(true)
        .trim(csv::Trim::All);
    if let Some(prefix) = options.comment_prefix {
        if prefix.is_ascii() {
            builder.comment(Some(prefix as u8));
        }
    }

    let mut reader = builder.from_reader(text.as_bytes());
    let mut records = Vec::new();
    for record in reader.records() {
        let record = record?;
        records.push(record.iter().map(|s| s.to_string()).collect::<Vec<_>>());
    }

    if records.is_empty() {
        return Err(FlightDataError::EmptyFile);
    }

    Ok(Tokenised {
        records,
        delimiter,
        delimiter_was_detected,
    })
}

/// Column count implied by the header row and every data row.
fn resolve_column_count(records: &[Vec<String>], has_header: bool) -> usize {
    let start = usize::from(has_header && !records.is_empty());
    let mut count = if has_header {
        records.first().map(|r| r.len()).unwrap_or(0)
    } else {
        0
    };
    for row in &records[start.min(records.len())..] {
        count = count.max(row.len());
    }
    count
}

fn build_headers(records: &[Vec<String>], has_header: bool, column_count: usize) -> Vec<String> {
    let mut headers = Vec::with_capacity(column_count);
    for index in 0..column_count {
        let declared = if has_header {
            records
                .first()
                .and_then(|row| row.get(index))
                .map(|s| s.trim().to_string())
                .unwrap_or_default()
        } else {
            String::new()
        };
        if declared.is_empty() {
            headers.push(format!("column_{index}"));
        } else {
            headers.push(declared);
        }
    }
    headers
}

/// Inspect a CSV document without building numeric columns.
///
/// Auto-detects the delimiter when the caller left it at the `,` default and the
/// first line contains more of another candidate, and decides whether the first
/// row is a header by checking whether it parses as numbers.
pub fn preview_csv(text: &str, options: &CsvImportOptions) -> Result<CsvPreview, FlightDataError> {
    let tokenised = tokenise(text, options)?;
    let records = &tokenised.records;

    let has_header = options.has_header
        && records
            .first()
            .map(|row| row_looks_like_header(row, options))
            .unwrap_or(false);

    let column_count = resolve_column_count(records, has_header);
    let headers = build_headers(records, has_header, column_count);
    let data_start = usize::from(has_header);

    let available_rows = records.len().saturating_sub(data_start);
    let limit = options
        .max_rows
        .map(|limit| limit.min(PREVIEW_SCAN_LIMIT))
        .unwrap_or(PREVIEW_SCAN_LIMIT);
    let row_count_preview = available_rows.min(limit);

    let sample_rows = records[data_start..]
        .iter()
        .take(PREVIEW_SAMPLE_ROWS)
        .map(|row| {
            let mut padded = row.clone();
            padded.resize(column_count, String::new());
            padded
        })
        .collect::<Vec<_>>();

    let mut warnings = Vec::new();
    if tokenised.delimiter_was_detected {
        warnings.push(format!(
            "Delimiter '{}' was detected from the first line; the requested ',' did not appear more often.",
            delimiter_label(tokenised.delimiter)
        ));
    }
    if options.has_header && !has_header {
        warnings.push(
            "The first row parses as numbers, so it was treated as data rather than a header."
                .to_string(),
        );
    }
    if has_header {
        warnings.push(format!(
            "The first row was treated as a header with {} column(s).",
            column_count
        ));
    }
    if let Some(max_rows) = options.max_rows {
        if available_rows > max_rows {
            warnings.push(format!(
                "The file has {available_rows} data rows; only the first {max_rows} will be imported."
            ));
        }
    }
    if column_count == 0 {
        warnings.push("No columns were found in the first rows.".to_string());
    }

    Ok(CsvPreview {
        headers,
        delimiter: tokenised.delimiter,
        detected_has_header: has_header,
        row_count_preview,
        sample_rows,
        warnings,
    })
}

/// Parse a CSV document into numeric columns.
///
/// Unparsable cells become `NaN` and increment `unparsable_cells`; absent cells
/// become `NaN` and increment `missing_cells`. `max_rows` truncates the table
/// rather than failing, so a preview and an import agree on the first rows.
pub fn parse_csv(text: &str, options: &CsvImportOptions) -> Result<RawTable, FlightDataError> {
    let tokenised = tokenise(text, options)?;
    let records = &tokenised.records;

    let has_header = options.has_header
        && records
            .first()
            .map(|row| row_looks_like_header(row, options))
            .unwrap_or(false);

    let column_count = resolve_column_count(records, has_header);
    let headers = build_headers(records, has_header, column_count);
    let data_start = usize::from(has_header);

    let source_lines = records.len();

    let mut columns: Vec<Vec<Real>> = vec![Vec::new(); column_count];
    let mut missing_cells = 0usize;
    let mut unparsable_cells = 0usize;

    let row_limit = options.max_rows.unwrap_or(usize::MAX);
    for row in records[data_start..].iter().take(row_limit) {
        for (index, column) in columns.iter_mut().enumerate() {
            match row.get(index) {
                Some(cell) if !cell.trim().is_empty() => match parse_cell(cell, options) {
                    Some(value) => column.push(value),
                    None => {
                        unparsable_cells += 1;
                        column.push(Real::NAN);
                    }
                },
                _ => {
                    missing_cells += 1;
                    column.push(Real::NAN);
                }
            }
        }
    }

    Ok(RawTable {
        headers,
        columns,
        missing_cells,
        unparsable_cells,
        source_lines,
    })
}

/// Parse a CSV document and fail when it exceeds `max_rows`.
///
/// [`parse_csv`] truncates, which is what a preview or an interactive import
/// wants. A batch or automated import wants to know that data was left behind,
/// so it calls this instead.
pub fn parse_csv_strict(
    text: &str,
    options: &CsvImportOptions,
) -> Result<RawTable, FlightDataError> {
    if let Some(limit) = options.max_rows {
        let mut probe = options.clone();
        probe.max_rows = None;
        let full = parse_csv(text, &probe)?;
        if full.row_count() > limit {
            return Err(FlightDataError::TooManyRows { limit });
        }
    }
    parse_csv(text, options)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts() -> CsvImportOptions {
        CsvImportOptions::default()
    }

    #[test]
    fn default_options_match_the_documented_values() {
        let o = opts();
        assert_eq!(o.delimiter, b',');
        assert!(o.has_header);
        assert_eq!(o.timestamp_unit, TimestampUnit::Seconds);
        assert!(o.timestamp_column.is_none());
        assert!(o.comment_prefix.is_none());
        assert!(o.max_rows.is_none());
        assert!(!o.decimal_comma);
    }

    #[test]
    fn empty_input_is_rejected() {
        let err = parse_csv("", &opts()).unwrap_err();
        assert!(matches!(err, FlightDataError::EmptyFile));
        let err = preview_csv("   \n  \n", &opts()).unwrap_err();
        assert!(matches!(err, FlightDataError::EmptyFile));
    }

    #[test]
    fn comma_delimited_file_with_header_parses() {
        let text = "time,ax,ay\n0.0,1.0,2.0\n0.01,3.0,4.0\n";
        let table = parse_csv(text, &opts()).unwrap();
        assert_eq!(table.headers, vec!["time", "ax", "ay"]);
        assert_eq!(table.row_count(), 2);
        assert_eq!(table.columns[0], vec![0.0, 0.01]);
        assert_eq!(table.columns[2], vec![2.0, 4.0]);
        assert_eq!(table.missing_cells, 0);
        assert_eq!(table.unparsable_cells, 0);
        assert_eq!(table.source_lines, 3);
    }

    #[test]
    fn delimiter_is_detected_when_another_delimiter_dominates() {
        let text = "time;ax;ay\n0,0;1,0;2,0\n0,1;3,0;4,0\n";
        let preview = preview_csv(text, &opts()).unwrap();
        assert_eq!(preview.delimiter, b';');
        assert!(preview
            .warnings
            .iter()
            .any(|w| w.contains("Delimiter ';' was detected")));
        assert_eq!(preview.headers, vec!["time", "ax", "ay"]);
    }

    #[test]
    fn tab_and_pipe_delimiters_are_detected() {
        let tab = preview_csv("time\tax\n0\t1\n", &opts()).unwrap();
        assert_eq!(tab.delimiter, b'\t');
        assert!(tab.warnings.iter().any(|w| w.contains("tab")));

        let pipe = preview_csv("time|ax\n0|1\n", &opts()).unwrap();
        assert_eq!(pipe.delimiter, b'|');
    }

    #[test]
    fn an_explicit_delimiter_is_never_overridden() {
        let mut options = opts();
        options.delimiter = b';';
        let preview = preview_csv("time;ax,ay\n0;1\n", &options).unwrap();
        assert_eq!(preview.delimiter, b';');
        assert!(preview.warnings.iter().all(|w| !w.contains("detected")));
    }

    #[test]
    fn header_detection_follows_the_first_row() {
        let with_header = preview_csv("time,ax\n0,1\n", &opts()).unwrap();
        assert!(with_header.detected_has_header);
        assert_eq!(with_header.headers, vec!["time", "ax"]);

        let without_header = preview_csv("0,1\n2,3\n", &opts()).unwrap();
        assert!(!without_header.detected_has_header);
        assert_eq!(without_header.headers, vec!["column_0", "column_1"]);
        assert!(without_header
            .warnings
            .iter()
            .any(|w| w.contains("parses as numbers")));
    }

    #[test]
    fn has_header_false_keeps_the_first_row_as_data() {
        let mut options = opts();
        options.has_header = false;
        let table = parse_csv("time,ax\n0,1\n", &options).unwrap();
        assert_eq!(table.headers, vec!["column_0", "column_1"]);
        assert_eq!(table.row_count(), 2);
    }

    #[test]
    fn quoted_fields_keep_their_delimiter_and_quotes() {
        let text = "name,value\n\"a,b\",1\n\"say \"\"hi\"\"\",2\n";
        let table = parse_csv(text, &opts()).unwrap();
        assert_eq!(table.headers[0], "name");
        // Both data rows collapse to NaN because the value column is numeric but
        // the text column is not; the quoted delimiters must not split fields.
        assert_eq!(table.row_count(), 2);
        assert_eq!(table.unparsable_cells, 2);
        let preview = preview_csv(text, &opts()).unwrap();
        assert_eq!(preview.sample_rows[0][0], "a,b");
        assert_eq!(preview.sample_rows[1][0], "say \"hi\"");
    }

    #[test]
    fn decimal_comma_is_honoured() {
        let text = "time;ax\n0,5;1,25\n";
        let mut options = opts();
        options.decimal_comma = true;
        let table = parse_csv(text, &options).unwrap();
        assert_eq!(table.headers, vec!["time", "ax"]);
        assert!((table.columns[0][0] - 0.5).abs() < 1e-12);
        assert!((table.columns[1][0] - 1.25).abs() < 1e-12);
    }

    #[test]
    fn decimal_comma_handles_thousands_grouping() {
        let text = "time;ax\n1.234,5;2\n";
        let mut options = opts();
        options.decimal_comma = true;
        let table = parse_csv(text, &options).unwrap();
        assert!((table.columns[0][0] - 1234.5).abs() < 1e-9);
    }

    #[test]
    fn unparsable_cells_become_nan_and_are_counted() {
        let text = "time,ax\n0.0,oops\n0.1,2.0\n";
        let table = parse_csv(text, &opts()).unwrap();
        assert_eq!(table.unparsable_cells, 1);
        assert!(table.columns[1][0].is_nan());
        assert_eq!(table.columns[1][1], 2.0);
        assert_eq!(table.row_count(), 2);
    }

    #[test]
    fn missing_cells_are_counted_and_padded_to_nan() {
        let text = "time,ax,ay\n0.0,1.0,2.0\n0.1,3.0\n";
        let table = parse_csv(text, &opts()).unwrap();
        assert_eq!(table.missing_cells, 1);
        assert_eq!(table.row_count(), 2);
        assert!(table.columns[2][1].is_nan());
    }

    #[test]
    fn extra_cells_widen_the_table_instead_of_being_dropped() {
        let text = "time,ax\n0.0,1.0,9.0\n";
        let table = parse_csv(text, &opts()).unwrap();
        assert_eq!(table.headers.len(), 3);
        assert_eq!(table.headers[2], "column_2");
        assert_eq!(table.columns[2][0], 9.0);
        assert_eq!(table.missing_cells, 0);
    }

    #[test]
    fn max_rows_truncates_the_parsed_table() {
        let text = "time,ax\n0,1\n1,2\n2,3\n3,4\n";
        let mut options = opts();
        options.max_rows = Some(2);
        let table = parse_csv(text, &options).unwrap();
        assert_eq!(table.row_count(), 2);
        assert_eq!(table.columns[0], vec![0.0, 1.0]);
        assert_eq!(table.source_lines, 5);

        let preview = preview_csv(text, &options).unwrap();
        assert_eq!(preview.row_count_preview, 2);
        assert!(preview
            .warnings
            .iter()
            .any(|w| w.contains("only the first 2 will be imported")));
    }

    #[test]
    fn strict_parsing_reports_too_many_rows() {
        let text = "time,ax\n0,1\n1,2\n2,3\n";
        let mut options = opts();
        options.max_rows = Some(2);
        let err = parse_csv_strict(text, &options).unwrap_err();
        assert!(matches!(err, FlightDataError::TooManyRows { limit: 2 }));
        assert!(err.user_message().contains("more than 2 rows"));

        options.max_rows = Some(10);
        assert_eq!(parse_csv_strict(text, &options).unwrap().row_count(), 3);
    }

    #[test]
    fn preview_reports_headers_samples_and_counts() {
        let text = "time,ax\n0.0,1.5\n0.1,2.5\n0.2,3.5\n";
        let preview = preview_csv(text, &opts()).unwrap();
        assert_eq!(preview.headers, vec!["time", "ax"]);
        assert_eq!(preview.row_count_preview, 3);
        assert_eq!(preview.sample_rows.len(), 3);
        assert_eq!(preview.sample_rows[0], vec!["0.0", "1.5"]);
        assert!(preview.detected_has_header);
    }

    #[test]
    fn preview_limits_the_sample_rows_but_not_the_count() {
        let mut text = String::from("time,ax\n");
        for i in 0..50 {
            text.push_str(&format!("{i},1\n"));
        }
        let preview = preview_csv(&text, &opts()).unwrap();
        assert_eq!(preview.sample_rows.len(), PREVIEW_SAMPLE_ROWS);
        assert_eq!(preview.row_count_preview, 50);
    }

    #[test]
    fn comment_lines_are_skipped() {
        let text = "# generated by the logger\ntime,ax\n0.0,1.0\n";
        let mut options = opts();
        options.comment_prefix = Some('#');
        let table = parse_csv(text, &options).unwrap();
        assert_eq!(table.headers, vec!["time", "ax"]);
        assert_eq!(table.row_count(), 1);
        // The delimiter detector must skip the comment line too, because that
        // line contains no delimiter and would otherwise hide the real one.
        let spaced = "# note; with a semicolon\ntime,ax\n0.0,1.0\n";
        let preview = preview_csv(spaced, &options).unwrap();
        assert_eq!(preview.delimiter, b',');
    }

    #[test]
    fn single_column_file_keeps_the_requested_delimiter() {
        let preview = preview_csv("value\n1\n2\n", &opts()).unwrap();
        assert_eq!(preview.delimiter, b',');
        assert!(preview.warnings.iter().all(|w| !w.contains("detected")));
        assert_eq!(preview.headers, vec!["value"]);
    }

    #[test]
    fn ragged_rows_do_not_panic_or_drop_columns() {
        let text = "time,ax,ay\n0,1\n1,2,3,4\n";
        let table = parse_csv(text, &opts()).unwrap();
        assert_eq!(table.headers.len(), 4);
        assert_eq!(table.row_count(), 2);
        // The first row supplies two of four columns, so two cells are absent.
        assert_eq!(table.missing_cells, 2);
    }

    #[test]
    fn column_lookup_is_case_insensitive() {
        let table = parse_csv("Time,AX\n0,1\n", &opts()).unwrap();
        assert_eq!(table.column_index("time"), Some(0));
        assert_eq!(table.column_index("ax"), Some(1));
        assert_eq!(table.column_index("nope"), None);
        assert!(table.column(9).is_none());
    }
}
