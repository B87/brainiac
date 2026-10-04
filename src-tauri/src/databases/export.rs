//! Export… (SPEC.md, Databases: Results): every row of a statement, run
//! again read only, written to a CSV or JSON file chosen in the save dialog.
//! Values are whole here, never cut as they are for the window.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

use crate::models::{AppError, AppResult, ExportFormat};

/// A value as written to a file.
#[derive(Debug, Clone, PartialEq)]
pub enum ExportValue {
    Null,
    Bool(bool),
    /// A number as the database printed it, so `numeric` keeps every digit.
    Number(String),
    /// JSON text, embedded as JSON in a JSON export.
    Json(String),
    Text(String),
}

/// Where exported rows go.
pub trait RowSink {
    fn columns(&mut self, names: &[String]) -> AppResult<()>;
    fn row(&mut self, values: &[ExportValue]) -> AppResult<()>;
    fn finish(&mut self) -> AppResult<()>;
}

fn write_error(e: std::io::Error) -> AppError {
    AppError::io("The export file could not be written.").with_details(e.to_string())
}

/// A file being written; it replaces the target only when complete.
pub struct FileSink {
    out: BufWriter<File>,
    format: ExportFormat,
    names: Vec<String>,
    rows: u64,
    temp: std::path::PathBuf,
    target: std::path::PathBuf,
}

impl FileSink {
    pub fn create(path: &Path, format: ExportFormat) -> AppResult<Self> {
        if !path.is_absolute() {
            return Err(AppError::validation("Choose where to save the export."));
        }
        let mut name = path.file_name().unwrap_or_default().to_os_string();
        name.push(".brainiac-partial");
        let temp = path.with_file_name(name);
        let file = File::create(&temp).map_err(write_error)?;
        Ok(FileSink {
            out: BufWriter::new(file),
            format,
            names: Vec::new(),
            rows: 0,
            temp,
            target: path.to_path_buf(),
        })
    }

    /// Where the file is written until it is complete.
    pub fn partial_path(&self) -> std::path::PathBuf {
        self.temp.clone()
    }

    /// Remove the partial file of an export that failed.
    pub fn abandon(self) {
        let FileSink { out, temp, .. } = self;
        drop(out);
        let _ = std::fs::remove_file(temp);
    }
}

pub fn csv_field(text: &str) -> String {
    if text.contains([',', '"', '\n', '\r']) || text.starts_with(' ') || text.ends_with(' ') {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text.to_string()
    }
}

fn json_value(value: &ExportValue) -> String {
    match value {
        ExportValue::Null => "null".into(),
        ExportValue::Bool(b) => b.to_string(),
        ExportValue::Number(n) => {
            // JSON has no NaN or Infinity.
            if n.parse::<f64>().is_ok_and(f64::is_finite) {
                n.clone()
            } else {
                serde_json::Value::String(n.clone()).to_string()
            }
        }
        ExportValue::Json(j) => match serde_json::from_str::<serde_json::Value>(j) {
            Ok(v) => v.to_string(),
            Err(_) => serde_json::Value::String(j.clone()).to_string(),
        },
        ExportValue::Text(t) => serde_json::Value::String(t.clone()).to_string(),
    }
}

impl RowSink for FileSink {
    fn columns(&mut self, names: &[String]) -> AppResult<()> {
        self.names = names.to_vec();
        match self.format {
            ExportFormat::Csv => {
                let header: Vec<String> = names.iter().map(|n| csv_field(n)).collect();
                writeln!(self.out, "{}", header.join(",")).map_err(write_error)
            }
            ExportFormat::Json => write!(self.out, "[").map_err(write_error),
        }
    }

    fn row(&mut self, values: &[ExportValue]) -> AppResult<()> {
        match self.format {
            ExportFormat::Csv => {
                let fields: Vec<String> = values
                    .iter()
                    .map(|v| match v {
                        ExportValue::Null => String::new(),
                        ExportValue::Bool(b) => b.to_string(),
                        ExportValue::Number(n) | ExportValue::Json(n) | ExportValue::Text(n) => {
                            csv_field(n)
                        }
                    })
                    .collect();
                writeln!(self.out, "{}", fields.join(",")).map_err(write_error)?;
            }
            ExportFormat::Json => {
                let fields: Vec<String> = self
                    .names
                    .iter()
                    .zip(values)
                    .map(|(name, v)| {
                        format!(
                            "{}:{}",
                            serde_json::Value::String(name.clone()),
                            json_value(v)
                        )
                    })
                    .collect();
                let lead = if self.rows == 0 { "\n" } else { ",\n" };
                write!(self.out, "{lead}{{{}}}", fields.join(",")).map_err(write_error)?;
            }
        }
        self.rows += 1;
        Ok(())
    }

    fn finish(&mut self) -> AppResult<()> {
        if self.format == ExportFormat::Json {
            write!(self.out, "\n]\n").map_err(write_error)?;
        }
        self.out.flush().map_err(write_error)?;
        std::fs::rename(&self.temp, &self.target).map_err(write_error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_and_json_files_hold_every_value() {
        let dir = tempfile::tempdir().unwrap();
        let rows = [
            vec![
                ExportValue::Number("1".into()),
                ExportValue::Text("a, \"b\"".into()),
                ExportValue::Json("{\"x\": 1}".into()),
                ExportValue::Null,
            ],
            vec![
                ExportValue::Number("NaN".into()),
                ExportValue::Text("plain".into()),
                ExportValue::Bool(true),
                ExportValue::Number("123456789.000000001".into()),
            ],
        ];
        let names = ["id", "name", "meta", "amount"].map(String::from);
        for format in [ExportFormat::Csv, ExportFormat::Json] {
            let path = dir.path().join(format!("out.{format:?}"));
            let mut sink = FileSink::create(&path, format).unwrap();
            sink.columns(&names).unwrap();
            for row in &rows {
                sink.row(row).unwrap();
            }
            sink.finish().unwrap();
            let text = std::fs::read_to_string(&path).unwrap();
            match format {
                ExportFormat::Csv => assert_eq!(
                    text,
                    "id,name,meta,amount\n1,\"a, \"\"b\"\"\",\"{\"\"x\"\": 1}\",\nNaN,plain,true,123456789.000000001\n"
                ),
                ExportFormat::Json => {
                    let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
                    assert_eq!(parsed[0]["meta"]["x"], 1);
                    assert_eq!(parsed[1]["id"], "NaN");
                    assert!(text.contains("123456789.000000001"));
                }
            }
        }
    }
}
