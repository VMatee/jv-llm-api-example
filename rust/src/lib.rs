//! Reusable async client for the public JV LLM API.
//!
//! No server protocol extensions, automatic submission retries, cookie storage,
//! or credential logging. Call [`JvClient::logout`] before dropping the client.
mod attachments;
mod auth;
mod client;
mod error;
mod files;
mod jobs;
mod responses;
mod types;

pub use attachments::{InputContent, StagedFile, local_image};
pub use client::{ClientConfig, DEFAULT_BASE_URL, JvClient, validate_base_url};
pub use error::{Error, Result, parse_retry_after};
pub use files::{safe_filename, validate_download_url};
pub use responses::{
    AgentResponse, CODEX_APPLY_PATCH_LARK, CustomTool, CustomToolFormat, FunctionCallOutputValue,
    FunctionOutputContent, FunctionTool, ResponseError, ResponseInput, ResponseOutput,
    ResponseRequest, ResponseStatus, ResponseTool, ToolChoice,
};
pub use types::{Job, JobResponse, JobStatus, JvJobRequest, ResponseFile};
