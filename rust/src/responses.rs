use crate::{Error, JvClient, Result, client::read_json, error::http_error, types::validate_id};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;
use tokio::time::{Instant, sleep, timeout_at};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResponseStatus {
    Queued,
    InProgress,
    Completed,
    Failed,
}

impl ResponseStatus {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ResponseError {
    pub code: String,
    pub message: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ResponseOutput {
    Message {
        id: String,
        status: ResponseStatus,
        role: String,
        content: Vec<OutputContent>,
    },
    FunctionCall {
        id: String,
        call_id: String,
        status: ResponseStatus,
        name: String,
        arguments: String,
    },
    CustomToolCall {
        id: String,
        call_id: String,
        name: String,
        input: String,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OutputContent {
    OutputText {
        text: String,
        #[serde(default)]
        annotations: Vec<Value>,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AgentResponse {
    pub id: String,
    pub object: String,
    pub status: ResponseStatus,
    #[serde(default)]
    pub output: Vec<ResponseOutput>,
    #[serde(default)]
    pub error: Option<ResponseError>,
}

impl AgentResponse {
    fn validate(&self, expected_id: Option<&str>) -> Result<()> {
        validate_id(&self.id).map_err(|_| Error::MalformedResponse)?;
        if expected_id.is_some_and(|id| id != self.id) || self.object != "response" {
            return Err(Error::MalformedResponse);
        }
        if self.status == ResponseStatus::Completed {
            if self.output.len() != 1 || self.error.is_some() {
                return Err(Error::MalformedResponse);
            }
        } else if !self.output.is_empty() {
            return Err(Error::MalformedResponse);
        }
        Ok(())
    }

    pub fn output_text(&self) -> Result<&str> {
        if self.status != ResponseStatus::Completed || self.output.len() != 1 {
            return Err(Error::MalformedResponse);
        }
        match &self.output[0] {
            ResponseOutput::Message {
                status: ResponseStatus::Completed,
                role,
                content,
                ..
            } if role == "assistant" && content.len() == 1 => match &content[0] {
                OutputContent::OutputText { text, .. } => Ok(text),
            },
            _ => Err(Error::MalformedResponse),
        }
    }

    pub fn function_call(&self) -> Result<(&str, &str, &str)> {
        if self.status != ResponseStatus::Completed || self.output.len() != 1 {
            return Err(Error::MalformedResponse);
        }
        match &self.output[0] {
            ResponseOutput::FunctionCall {
                call_id,
                status: ResponseStatus::Completed,
                name,
                arguments,
                ..
            } => Ok((call_id, name, arguments)),
            _ => Err(Error::MalformedResponse),
        }
    }

    pub fn custom_tool_call(&self) -> Result<(&str, &str)> {
        if self.status != ResponseStatus::Completed || self.output.len() != 1 {
            return Err(Error::MalformedResponse);
        }
        match &self.output[0] {
            ResponseOutput::CustomToolCall {
                id,
                call_id,
                name,
                input,
                ..
            } if name == "apply_patch" && !input.is_empty() && input.len() <= 32 * 1024 => {
                validate_id(id).map_err(|_| Error::MalformedResponse)?;
                validate_id(call_id).map_err(|_| Error::MalformedResponse)?;
                Ok((call_id, input))
            }
            _ => Err(Error::MalformedResponse),
        }
    }
}

#[derive(Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FunctionOutputContent {
    InputImage { image_url: String, detail: String },
}

// Image data URLs must not appear through accidental debug logging.
impl std::fmt::Debug for FunctionOutputContent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("InputImage([redacted])")
    }
}

impl TryFrom<crate::InputContent> for FunctionOutputContent {
    type Error = Error;

    fn try_from(value: crate::InputContent) -> Result<Self> {
        match value {
            crate::InputContent::InputImage { image_url, detail } => {
                Ok(Self::InputImage { image_url, detail })
            }
            _ => Err(Error::InvalidInput("tool result requires input_image")),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(untagged)]
pub enum FunctionCallOutputValue {
    Text(String),
    Content(Vec<FunctionOutputContent>),
}

#[derive(Clone, Debug, Serialize)]
#[serde(untagged)]
pub enum ResponseInput {
    ContentMessage {
        role: String,
        content: Vec<crate::InputContent>,
    },
    Message {
        role: String,
        content: String,
    },
    FunctionCallOutput {
        r#type: String,
        call_id: String,
        output: FunctionCallOutputValue,
    },
    CustomToolCallOutput {
        r#type: String,
        call_id: String,
        output: String,
    },
}

#[derive(Clone, Debug, Serialize)]
pub struct FunctionTool {
    pub r#type: String,
    pub name: String,
    pub description: String,
    pub strict: bool,
    pub parameters: Value,
}

#[derive(Clone, Debug, Serialize)]
pub struct CustomToolFormat {
    pub r#type: String,
    pub syntax: String,
    pub definition: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct CustomTool {
    pub r#type: String,
    pub name: String,
    pub description: String,
    pub format: CustomToolFormat,
}

pub const CODEX_APPLY_PATCH_LARK: &str =
    include_str!("../../examples/codex-0.149.1-apply-patch.lark");

impl CustomTool {
    pub fn codex_0_149_1_apply_patch() -> Self {
        Self {
            r#type: "custom".into(),
            name: "apply_patch".into(),
            description: "The `apply_patch` tool can be used to edit files. This is a FREEFORM tool, so do not wrap the patch in JSON.".into(),
            format: CustomToolFormat {
                r#type: "grammar".into(),
                syntax: "lark".into(),
                definition: CODEX_APPLY_PATCH_LARK.into(),
            },
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(untagged)]
pub enum ResponseTool {
    Function(FunctionTool),
    Custom(CustomTool),
}

impl From<FunctionTool> for ResponseTool {
    fn from(value: FunctionTool) -> Self {
        Self::Function(value)
    }
}

impl From<CustomTool> for ResponseTool {
    fn from(value: CustomTool) -> Self {
        Self::Custom(value)
    }
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolChoice {
    Auto,
    None,
    Required,
}

#[derive(Clone, Debug, Serialize)]
pub struct ResponseRequest {
    pub model: String,
    pub background: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_response_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    pub input: Vec<ResponseInput>,
    pub tools: Vec<ResponseTool>,
    pub tool_choice: ToolChoice,
    pub parallel_tool_calls: bool,
    pub store: bool,
    pub stream: bool,
}

impl ResponseRequest {
    pub fn text(question: impl Into<String>) -> Self {
        Self {
            model: "jv-ai".into(),
            background: true,
            previous_response_id: None,
            instructions: None,
            input: vec![ResponseInput::Message {
                role: "user".into(),
                content: question.into(),
            }],
            tools: vec![],
            tool_choice: ToolChoice::None,
            parallel_tool_calls: false,
            store: true,
            stream: false,
        }
    }

    pub fn continuation(
        previous_response_id: impl Into<String>,
        call_id: impl Into<String>,
        output: impl Into<String>,
    ) -> Self {
        Self {
            model: "jv-ai".into(),
            background: true,
            previous_response_id: Some(previous_response_id.into()),
            instructions: Some(
                "Use the trusted tool result and answer the original request.".into(),
            ),
            input: vec![ResponseInput::FunctionCallOutput {
                r#type: "function_call_output".into(),
                call_id: call_id.into(),
                output: FunctionCallOutputValue::Text(output.into()),
            }],
            tools: vec![],
            tool_choice: ToolChoice::None,
            parallel_tool_calls: false,
            store: true,
            stream: false,
        }
    }

    pub fn image_tool_continuation(
        previous_response_id: impl Into<String>,
        call_id: impl Into<String>,
        images: Vec<FunctionOutputContent>,
    ) -> Self {
        let mut request = Self::continuation(previous_response_id, call_id, "");
        if let ResponseInput::FunctionCallOutput { output, .. } = &mut request.input[0] {
            *output = FunctionCallOutputValue::Content(images);
        }
        request
    }

    pub fn custom_tool_continuation(
        previous_response_id: impl Into<String>,
        call_id: impl Into<String>,
        output: impl Into<String>,
    ) -> Self {
        Self {
            model: "jv-ai".into(),
            background: true,
            previous_response_id: Some(previous_response_id.into()),
            instructions: Some(
                "Use the client tool result and answer the original request.".into(),
            ),
            input: vec![ResponseInput::CustomToolCallOutput {
                r#type: "custom_tool_call_output".into(),
                call_id: call_id.into(),
                output: output.into(),
            }],
            tools: vec![],
            tool_choice: ToolChoice::None,
            parallel_tool_calls: false,
            store: true,
            stream: false,
        }
    }

    fn validate(&self) -> Result<()> {
        if self.model != "jv-ai"
            || !self.background
            || self.stream
            || !self.store
            || self.parallel_tool_calls
        {
            return Err(Error::InvalidInput("unsupported Responses pilot option"));
        }
        if self.input.is_empty() || self.input.len() > 16 || self.tools.len() > 16 {
            return Err(Error::InvalidInput("invalid Responses input or tool count"));
        }
        for input in &self.input {
            match input {
                ResponseInput::FunctionCallOutput {
                    output: FunctionCallOutputValue::Content(images),
                    ..
                } if images.is_empty() || images.len() > 4 => {
                    return Err(Error::InvalidInput("invalid tool-result image count"));
                }
                ResponseInput::FunctionCallOutput {
                    call_id,
                    output: FunctionCallOutputValue::Content(images),
                    ..
                } => {
                    validate_id(call_id)?;
                    for image in images {
                        match image {
                            FunctionOutputContent::InputImage { image_url, detail }
                                if matches!(detail.as_str(), "auto" | "high")
                                    && image_url.len() <= 6_990_508 + 32
                                    && [
                                        "data:image/png;base64,",
                                        "data:image/jpeg;base64,",
                                        "data:image/webp;base64,",
                                    ]
                                    .iter()
                                    .any(|prefix| image_url.starts_with(prefix)) => {}
                            _ => {
                                return Err(Error::InvalidInput("unsupported tool-result image"));
                            }
                        }
                    }
                }
                ResponseInput::FunctionCallOutput { call_id, .. } => {
                    validate_id(call_id)?;
                }
                ResponseInput::CustomToolCallOutput {
                    call_id, output, ..
                } => {
                    validate_id(call_id)?;
                    if output.len() > 32 * 1024 {
                        return Err(Error::InvalidInput("custom tool result too large"));
                    }
                }
                _ => {}
            }
        }
        for tool in &self.tools {
            if let ResponseTool::Custom(custom) = tool
                && (custom.r#type != "custom"
                    || custom.name != "apply_patch"
                    || custom.format.r#type != "grammar"
                    || custom.format.syntax != "lark"
                    || custom.format.definition != CODEX_APPLY_PATCH_LARK)
            {
                return Err(Error::InvalidInput("unsupported custom tool"));
            }
        }
        if let Some(id) = &self.previous_response_id {
            validate_id(id)?;
        }
        Ok(())
    }
}

impl JvClient {
    /// Submit exactly once. Callers must retain the key for a safe explicit retry.
    pub async fn submit_response(
        &self,
        request: &ResponseRequest,
        idempotency_key: &str,
    ) -> Result<AgentResponse> {
        request.validate()?;
        if idempotency_key.is_empty()
            || idempotency_key.len() > 128
            || !idempotency_key
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'.' | b':' | b'-'))
        {
            return Err(Error::InvalidInput("invalid idempotency key"));
        }
        let response = self
            .authenticated(Method::POST, self.endpoint("/v1/responses")?)?
            .header("Idempotency-Key", idempotency_key)
            .json(request)
            .send()
            .await
            .map_err(|_| Error::ResponseSubmissionUncertain {
                status: None,
                idempotency_key: idempotency_key.into(),
            })?;
        let status = response.status().as_u16();
        if status != 200 && status != 202 {
            if (400..500).contains(&status) && status != 408 {
                return Err(http_error(&response));
            }
            return Err(Error::ResponseSubmissionUncertain {
                status: Some(status),
                idempotency_key: idempotency_key.into(),
            });
        }
        let value: AgentResponse =
            read_json(response)
                .await
                .map_err(|_| Error::ResponseSubmissionUncertain {
                    status: Some(status),
                    idempotency_key: idempotency_key.into(),
                })?;
        value
            .validate(None)
            .map_err(|_| Error::ResponseSubmissionUncertain {
                status: Some(status),
                idempotency_key: idempotency_key.into(),
            })?;
        Ok(value)
    }

    pub async fn get_response(&self, response_id: &str) -> Result<AgentResponse> {
        validate_id(response_id)?;
        let response = self
            .authenticated(
                Method::GET,
                self.endpoint(&format!("/v1/responses/{response_id}"))?,
            )?
            .send()
            .await
            .map_err(|_| Error::Network)?;
        if response.status() != 200 {
            return Err(http_error(&response));
        }
        let value: AgentResponse = read_json(response).await?;
        value.validate(Some(response_id))?;
        Ok(value)
    }

    pub async fn wait_for_response(&self, response_id: &str) -> Result<AgentResponse> {
        validate_id(response_id)?;
        let deadline = Instant::now() + self.config.wait_timeout;
        let timed_out = || Error::ResponseWaitTimeout {
            response_id: response_id.into(),
        };
        let mut errors = 0u32;
        loop {
            if Instant::now() >= deadline {
                return Err(timed_out());
            }
            let result = timeout_at(deadline, self.get_response(response_id))
                .await
                .map_err(|_| timed_out())?;
            let delay = match result {
                Ok(value) => {
                    errors = 0;
                    if value.status.is_terminal() {
                        return Ok(value);
                    }
                    self.config.poll_interval
                }
                Err(error) => {
                    errors += 1;
                    if !error.retryable_poll() || errors >= self.config.max_poll_errors {
                        return Err(error);
                    }
                    let backoff = self
                        .config
                        .poll_interval
                        .saturating_mul(1 << (errors - 1).min(5))
                        .min(Duration::from_secs(30));
                    let retry_after = match error {
                        Error::Http { retry_after, .. } => retry_after.unwrap_or_default(),
                        _ => Duration::ZERO,
                    };
                    backoff.max(retry_after)
                }
            };
            let remaining = deadline.saturating_duration_since(Instant::now());
            if delay >= remaining {
                sleep(remaining).await;
                return Err(timed_out());
            }
            sleep(delay).await;
        }
    }
}
