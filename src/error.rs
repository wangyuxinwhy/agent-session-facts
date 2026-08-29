use std::fmt;

use serde::Serialize;

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ErrorCategory {
    InvalidInput,
    Io,
    Unsupported,
    Internal,
}

#[derive(Debug)]
pub(crate) struct AppError {
    category: ErrorCategory,
    message: String,
}

impl AppError {
    pub(crate) fn invalid(message: impl Into<String>) -> Self {
        Self::new(ErrorCategory::InvalidInput, message)
    }

    pub(crate) fn io(message: impl Into<String>) -> Self {
        Self::new(ErrorCategory::Io, message)
    }

    pub(crate) fn unsupported(message: impl Into<String>) -> Self {
        Self::new(ErrorCategory::Unsupported, message)
    }

    pub(crate) fn internal(message: impl Into<String>) -> Self {
        Self::new(ErrorCategory::Internal, message)
    }

    pub(crate) fn exit_code(&self) -> i32 {
        match self.category {
            ErrorCategory::InvalidInput | ErrorCategory::Unsupported => 2,
            ErrorCategory::Io | ErrorCategory::Internal => 1,
        }
    }

    fn new(category: ErrorCategory, message: impl Into<String>) -> Self {
        Self {
            category,
            message: message.into(),
        }
    }
}

impl fmt::Display for AppError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for AppError {}

#[derive(Serialize)]
struct ErrorEnvelope<'a> {
    schema_version: u32,
    #[serde(rename = "type")]
    kind: &'static str,
    category: ErrorCategory,
    message: &'a str,
}

pub(crate) fn emit(error: &AppError) {
    let envelope = ErrorEnvelope {
        schema_version: 1,
        kind: "agent_session_facts.error",
        category: error.category,
        message: error.message.as_str(),
    };
    if serde_json::to_writer(std::io::stderr().lock(), &envelope).is_ok() {
        eprintln!();
    } else {
        eprintln!("agent-session-facts failed: {error}");
    }
}
