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
    pub tools: Vec<FunctionTool>,
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
