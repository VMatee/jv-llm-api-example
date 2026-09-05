use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Only current text, current attachments, and an optional owned conversation ID.
#[derive(Clone, Debug)]
pub struct JvJobRequest {
    pub text: String,
    pub conversation_id: Option<String>,
    pub files: Vec<PathBuf>,
}

impl JvJobRequest {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            conversation_id: None,
            files: Vec::new(),
        }
    }

    pub fn validate(&self) -> Result<()> {
        if self.text.trim().is_empty() {
            return Err(Error::InvalidInput("question text must not be empty"));
        }
        if let Some(id) = &self.conversation_id {
            validate_id(id)?;
        }
        Ok(())
    }
}

/// The central API's statuses. `result_ready` is a flag, not terminal success.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Queued,
    Dispatching,
    WaitingForProvider,
    Running,
    WaitingForAuth,
    Succeeded,
    Failed,
}

impl JobStatus {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ResponseFile {
    pub name: String,
    pub url: String,
    pub size_bytes: u64,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub content_type: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct JobResponse {
    #[serde(default)]
    pub files: Vec<ResponseFile>,
}

/// Selected public fields; unrelated fields (including admin routing) are ignored.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Job {
    pub id: String,
    pub conversation_id: String,
    pub status: JobStatus,
    #[serde(default)]
    pub conversation_turn: Option<u64>,
    #[serde(default)]
    pub phase: Option<String>,
    #[serde(default)]
    pub queue_position: Option<u64>,
    #[serde(default)]
    pub result_ready: bool,
    #[serde(default)]
    pub answer: Option<String>,
    #[serde(default)]
    pub response: JobResponse,
    #[serde(default)]
    pub error_code: Option<String>,
    #[serde(default)]
    pub error_message: Option<String>,
}

impl Job {
    pub(crate) fn validate(&self) -> Result<()> {
        validate_id(&self.id).map_err(|_| Error::MalformedResponse)?;
        validate_id(&self.conversation_id).map_err(|_| Error::MalformedResponse)?;
        Ok(())
    }
}

pub(crate) fn validate_id(id: &str) -> Result<()> {
    if id.is_empty()
        || id.len() > 200
        || !id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_'))
    {
        return Err(Error::InvalidInput(
            "ID must be a nonempty opaque alphanumeric, hyphen or underscore value",
        ));
    }
    Ok(())
}
