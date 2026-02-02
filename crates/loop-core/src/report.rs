//! Report TSV generation for bin/loop-analyze compatibility.
//!
//! Generates report.tsv files matching the format from bin/loop (Section 7.1).
//!
//! Columns: `timestamp_ms`, kind, iteration, `duration_ms`, `exit_code`, `output_bytes`,
//!          `output_lines`, `output_path`, message, `tasks_done`, `tasks_total`

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

/// A single row in the report.tsv file.
#[derive(Debug, Clone)]
pub struct ReportRow {
    /// Unix timestamp in milliseconds.
    pub timestamp_ms: i64,
    /// Event kind (e.g., `RUN_START`, `ITERATION_END`).
    pub kind: String,
    /// Iteration label (e.g., "1", "1R1", "2").
    pub iteration: String,
    /// Duration in milliseconds (optional).
    pub duration_ms: Option<u64>,
    /// Exit code (optional).
    pub exit_code: Option<i32>,
    /// Output size in bytes (optional).
    pub output_bytes: Option<u64>,
    /// Output line count (optional).
    pub output_lines: Option<u64>,
    /// Path to output file (optional).
    pub output_path: Option<String>,
    /// Message field for additional info.
    pub message: String,
    /// Number of completed tasks.
    pub tasks_done: Option<u32>,
    /// Total number of tasks.
    pub tasks_total: Option<u32>,
}

impl ReportRow {
    /// Create a new report row with required fields.
    pub fn new(timestamp_ms: i64, kind: impl Into<String>) -> Self {
        Self {
            timestamp_ms,
            kind: kind.into(),
            iteration: String::new(),
            duration_ms: None,
            exit_code: None,
            output_bytes: None,
            output_lines: None,
            output_path: None,
            message: String::new(),
            tasks_done: None,
            tasks_total: None,
        }
    }

    /// Set the iteration label.
    pub fn with_iteration(mut self, iteration: impl Into<String>) -> Self {
        self.iteration = iteration.into();
        self
    }

    /// Set duration in milliseconds.
    pub fn with_duration_ms(mut self, duration_ms: u64) -> Self {
        self.duration_ms = Some(duration_ms);
        self
    }

    /// Set exit code.
    pub fn with_exit_code(mut self, exit_code: i32) -> Self {
        self.exit_code = Some(exit_code);
        self
    }

    /// Set output size.
    pub fn with_output(mut self, bytes: u64, lines: u64) -> Self {
        self.output_bytes = Some(bytes);
        self.output_lines = Some(lines);
        self
    }

    /// Set output path.
    pub fn with_output_path(mut self, path: impl Into<String>) -> Self {
        self.output_path = Some(path.into());
        self
    }

    /// Set message.
    pub fn with_message(mut self, message: impl Into<String>) -> Self {
        self.message = message.into();
        self
    }

    /// Set task progress.
    pub fn with_tasks(mut self, done: u32, total: u32) -> Self {
        self.tasks_done = Some(done);
        self.tasks_total = Some(total);
        self
    }

    /// Format as a TSV line.
    fn to_tsv_line(&self) -> String {
        let duration = self.duration_ms.map(|d| d.to_string()).unwrap_or_default();
        let exit_code = self.exit_code.map(|c| c.to_string()).unwrap_or_default();
        let output_bytes = self.output_bytes.map(|b| b.to_string()).unwrap_or_default();
        let output_lines = self.output_lines.map(|l| l.to_string()).unwrap_or_default();
        let output_path = self.output_path.as_deref().unwrap_or("");
        let tasks_done = self.tasks_done.map(|t| t.to_string()).unwrap_or_default();
        let tasks_total = self.tasks_total.map(|t| t.to_string()).unwrap_or_default();

        // Sanitize message to prevent TSV breakage
        let safe_message = sanitize_field(&self.message);

        format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            self.timestamp_ms,
            self.kind,
            self.iteration,
            duration,
            exit_code,
            output_bytes,
            output_lines,
            output_path,
            safe_message,
            tasks_done,
            tasks_total,
        )
    }
}

/// Sanitize a field value to prevent TSV breakage.
fn sanitize_field(value: &str) -> String {
    value.replace(['\t', '\n', '\r'], " ")
}

/// TSV header row.
const HEADER: &str =
    "timestamp_ms\tkind\titeration\tduration_ms\texit_code\toutput_bytes\toutput_lines\toutput_path\tmessage\ttasks_done\ttasks_total";

/// Writer for report.tsv files.
pub struct ReportWriter {
    writer: BufWriter<File>,
}

impl std::fmt::Debug for ReportWriter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReportWriter")
            .field("writer", &"BufWriter<File>")
            .finish()
    }
}

impl ReportWriter {
    /// Create a new report writer, writing header if the file is new.
    pub fn new(path: &Path) -> std::io::Result<Self> {
        let exists = path.exists();
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        let mut writer = BufWriter::new(file);

        if !exists {
            writeln!(writer, "{HEADER}")?;
        }

        Ok(Self { writer })
    }

    /// Write a single report row.
    pub fn write_row(&mut self, row: &ReportRow) -> std::io::Result<()> {
        writeln!(self.writer, "{}", row.to_tsv_line())
    }

    /// Flush pending writes.
    pub fn flush(&mut self) -> std::io::Result<()> {
        self.writer.flush()
    }
}

/// Write multiple rows to a report file at once.
pub fn write_report(path: &Path, rows: &[ReportRow]) -> std::io::Result<()> {
    let file = File::create(path)?;
    let mut writer = BufWriter::new(file);

    writeln!(writer, "{HEADER}")?;
    for row in rows {
        writeln!(writer, "{}", row.to_tsv_line())?;
    }

    writer.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn report_row_to_tsv_line_with_all_fields() {
        let row = ReportRow::new(1769687293854, "RUN_START")
            .with_message("spec=/path/to/spec.md plan=/path/to/plan.md")
            .with_tasks(0, 28);

        let line = row.to_tsv_line();
        assert!(line.contains("1769687293854"));
        assert!(line.contains("RUN_START"));
        assert!(line.contains("spec=/path/to/spec.md"));
        assert!(line.contains("\t0\t28"));
    }

    #[test]
    fn report_row_to_tsv_line_with_minimal_fields() {
        let row = ReportRow::new(1769687294148, "ITERATION_START").with_iteration("1");

        let line = row.to_tsv_line();
        assert!(line.contains("ITERATION_START\t1"));
    }

    #[test]
    fn report_row_to_tsv_line_with_iteration_data() {
        let row = ReportRow::new(1769687952715, "ITERATION_END")
            .with_iteration("1")
            .with_duration_ms(658554)
            .with_exit_code(0)
            .with_output(84, 1)
            .with_output_path("/logs/iter-01.log")
            .with_tasks(4, 28);

        let line = row.to_tsv_line();
        assert!(line.contains("ITERATION_END\t1\t658554\t0\t84\t1"));
        assert!(line.contains("/logs/iter-01.log"));
    }

    #[test]
    fn sanitize_field_removes_control_chars() {
        let value = "line1\nline2\twith\ttabs\rcarriage";
        let sanitized = sanitize_field(value);
        assert!(!sanitized.contains('\t'));
        assert!(!sanitized.contains('\n'));
        assert!(!sanitized.contains('\r'));
    }

    #[test]
    fn report_writer_creates_header() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("report.tsv");

        {
            let mut writer = ReportWriter::new(&path).unwrap();
            let row = ReportRow::new(1000, "TEST_EVENT");
            writer.write_row(&row).unwrap();
            writer.flush().unwrap();
        }

        let content = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines[0], HEADER);
        assert!(lines[1].starts_with("1000\tTEST_EVENT"));
    }

    #[test]
    fn report_writer_appends_without_duplicate_header() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("report.tsv");

        // First write
        {
            let mut writer = ReportWriter::new(&path).unwrap();
            writer.write_row(&ReportRow::new(1000, "EVENT1")).unwrap();
            writer.flush().unwrap();
        }

        // Second write
        {
            let mut writer = ReportWriter::new(&path).unwrap();
            writer.write_row(&ReportRow::new(2000, "EVENT2")).unwrap();
            writer.flush().unwrap();
        }

        let content = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = content.lines().collect();

        // Should have header + 2 data rows
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0], HEADER);
        assert!(lines[1].contains("EVENT1"));
        assert!(lines[2].contains("EVENT2"));
    }

    #[test]
    fn write_report_creates_complete_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("report.tsv");

        let rows = vec![
            ReportRow::new(1000, "RUN_START").with_message("test run"),
            ReportRow::new(2000, "ITERATION_START").with_iteration("1"),
            ReportRow::new(3000, "ITERATION_END")
                .with_iteration("1")
                .with_duration_ms(1000)
                .with_exit_code(0),
        ];

        write_report(&path, &rows).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = content.lines().collect();

        assert_eq!(lines.len(), 4); // header + 3 rows
        assert_eq!(lines[0], HEADER);
        assert!(lines[1].contains("RUN_START"));
        assert!(lines[2].contains("ITERATION_START"));
        assert!(lines[3].contains("ITERATION_END"));
    }
}
