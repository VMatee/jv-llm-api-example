use crate::{
    Error, Job, JvClient, JvJobRequest, Result, client::read_json, error::http_error,
    types::validate_id,
};
use reqwest::{
    Method,
    multipart::{Form, Part},
};
use std::time::Duration;
use tokio::time::{Instant, sleep, timeout_at};

impl JvClient {
    /// Send exactly one multipart POST. An ambiguous response is never retried.
    pub async fn submit_job(&self, request: JvJobRequest) -> Result<Job> {
        let builder = self.authenticated(Method::POST, self.endpoint("/v1/jobs")?)?;
        request.validate()?;
        let conversation_id = request.conversation_id;
        let mut form = Form::new().text("text", request.text);
        if let Some(id) = &conversation_id {
            form = form.text("conversation_id", id.clone());
        }
        for path in request.files {
            let meta = tokio::fs::symlink_metadata(&path)
                .await
                .map_err(|_| Error::FileIo)?;
            if !meta.file_type().is_file() {
                return Err(Error::InvalidInput(
                    "attachment must be a regular, non-symlink file",
                ));
            }
            let name = path
                .file_name()
                .and_then(|v| v.to_str())
                .ok_or(Error::InvalidInput("attachment filename must be UTF-8"))?
                .to_owned();
            let file = tokio::fs::File::open(&path)
                .await
                .map_err(|_| Error::FileIo)?;
            let opened = file.metadata().await.map_err(|_| Error::FileIo)?;
            if !opened.is_file() {
                return Err(Error::InvalidInput("attachment must be a regular file"));
            }
            let part = Part::stream_with_length(file, opened.len())
                .file_name(name)
                .mime_str(
                    mime_guess::from_path(&path)
                        .first_or_octet_stream()
                        .as_ref(),
                )
                .map_err(|_| Error::InvalidInput("invalid attachment media type"))?;
            form = form.part("files", part);
        }
        let response = builder
            .multipart(form)
            .send()
            .await
            .map_err(|_| Error::SubmissionUncertain { status: None })?;
        let status = response.status().as_u16();
        if status != 202 {
            if (400..500).contains(&status) && status != 408 {
                return Err(http_error(&response));
            }
            return Err(Error::SubmissionUncertain {
                status: Some(status),
            });
        }
        let job: Job = read_json(response)
            .await
            .map_err(|_| Error::SubmissionUncertain { status: Some(202) })?;
        if job.validate().is_err()
            || conversation_id
                .as_ref()
                .is_some_and(|id| id != &job.conversation_id)
        {
            return Err(Error::SubmissionUncertain { status: Some(202) });
        }
        Ok(job)
    }

    /// One safe GET; wait_for_job adds bounded retries and a total deadline.
    pub async fn get_job(&self, job_id: &str) -> Result<Job> {
        validate_id(job_id)?;
        let response = self
            .authenticated(Method::GET, self.endpoint(&format!("/v1/jobs/{job_id}"))?)?
            .send()
            .await
            .map_err(|_| Error::Network)?;
        if response.status() != 200 {
            return Err(http_error(&response));
        }
        let job: Job = read_json(response).await?;
        job.validate()?;
        if job.id != job_id {
            return Err(Error::MalformedResponse);
        }
        Ok(job)
    }

    /// Returns terminal success OR failure; local timeout never cancels a server job.
    pub async fn wait_for_job(&self, job_id: &str) -> Result<Job> {
        self.wait_for_job_with_progress(job_id, |_| {}).await
    }

    pub async fn wait_for_job_with_progress<F>(&self, job_id: &str, mut progress: F) -> Result<Job>
    where
        F: FnMut(&Job),
    {
        validate_id(job_id)?;
        let deadline = Instant::now() + self.config.wait_timeout;
        let timed_out = || Error::WaitTimeout {
            job_id: job_id.to_owned(),
        };
        let mut errors = 0u32;
        let mut conversation = None;
        loop {
            if Instant::now() >= deadline {
                return Err(timed_out());
            }
            let result = timeout_at(deadline, self.get_job(job_id))
                .await
                .map_err(|_| timed_out())?;
            let delay = match result {
                Ok(job) => {
                    if conversation
                        .as_ref()
                        .is_some_and(|id| id != &job.conversation_id)
                    {
                        return Err(Error::MalformedResponse);
                    }
                    conversation = Some(job.conversation_id.clone());
                    errors = 0;
                    progress(&job);
                    if job.status.is_terminal() {
                        return Ok(job);
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
            // Never shorten Retry-After and issue an early request; expire locally instead.
            let remaining = deadline.saturating_duration_since(Instant::now());
            if delay >= remaining {
                sleep(remaining).await;
                return Err(timed_out());
            }
            sleep(delay).await;
        }
    }
}
