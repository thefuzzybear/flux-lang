/// A compile error with its byte offset in the source file.
#[derive(Debug, Clone)]
pub struct CompileErrorWithSpan {
    pub offset: usize,
    pub message: String,
}

/// Errors that can occur during CSV loading.
#[derive(Debug, thiserror::Error)]
pub enum CsvError {
    #[error("cannot open file: {0}")]
    FileAccess(std::io::Error),

    #[error("missing required columns: {0:?}")]
    MissingColumns(Vec<String>),

    #[error("row {row}: invalid value in column '{column}': expected numeric")]
    InvalidValue { row: usize, column: String },

    #[error("file contains no data rows")]
    EmptyFile,

    #[error("row {row}: timestamp value is required")]
    EmptyTimestamp { row: usize },
}

/// Top-level CLI error type.
#[derive(Debug, thiserror::Error)]
pub enum CliError {
    #[error("{}", format_compile_errors(.0))]
    Compile(Vec<CompileErrorWithSpan>),

    #[error("error: {0}")]
    Io(#[from] std::io::Error),

    #[error("error: {0}")]
    Csv(CsvError),

    #[error("error: {0}")]
    Runtime(String),
}

fn format_compile_errors(errors: &[CompileErrorWithSpan]) -> String {
    errors
        .iter()
        .map(|e| format!("offset {}: {}", e.offset, e.message))
        .collect::<Vec<_>>()
        .join("\n")
}
