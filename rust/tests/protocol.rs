use axum::{
    Router,
    body::{Body, to_bytes},
    extract::{Request, State},
    http::{HeaderMap, Response, StatusCode},
};
use jv_ai_client::{ClientConfig, Error, Job, JobStatus, JvClient, JvJobRequest};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::Duration,
};

struct Reply {
    status: u16,
    body: String,
    headers: Vec<(&'static str, String)>,
    delay: Duration,
}
impl Reply {
    fn json(status: u16, value: Value) -> Self {
        Self::raw(status, value.to_string())
    }
    fn raw(status: u16, body: impl Into<String>) -> Self {
        Self {
            status,
            body: body.into(),
            headers: vec![],
            delay: Duration::ZERO,
        }
    }
    fn header(mut self, key: &'static str, value: &str) -> Self {
        self.headers.push((key, value.into()));
        self
    }
}
struct Recorded {
    method: String,
    path: String,
    headers: HeaderMap,
    body: Vec<u8>,
}
#[derive(Clone)]
struct MockState {
    replies: Arc<Mutex<VecDeque<Reply>>>,
    requests: Arc<Mutex<Vec<Recorded>>>,
}
struct Mock {
    base: String,
    state: MockState,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Mock {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Mock {
    async fn start(replies: Vec<Reply>) -> Self {
        let state = MockState {
            replies: Arc::new(Mutex::new(replies.into())),
            requests: Arc::new(Mutex::new(Vec::new())),
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let router = Router::new().fallback(handler).with_state(state.clone());
        let task = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        Self { base, state, task }
    }
    fn client(&self) -> JvClient {
        JvClient::new(ClientConfig {
            base_url: self.base.clone(),
            poll_interval: Duration::from_millis(5),
            wait_timeout: Duration::from_secs(2),
            max_poll_errors: 3,
            ..ClientConfig::default()
        })
        .unwrap()
    }
    fn complete(&self) {
        assert!(
            self.state.replies.lock().unwrap().is_empty(),
            "not all expected requests were sent"
        );
    }
}
async fn handler(State(state): State<MockState>, request: Request) -> Response<Body> {
    let (parts, body) = request.into_parts();
    let body = to_bytes(body, 1024 * 1024).await.unwrap();
    state.requests.lock().unwrap().push(Recorded {
        method: parts.method.to_string(),
        path: parts.uri.path().into(),
        headers: parts.headers,
        body: body.to_vec(),
    });
    let reply = state.replies.lock().unwrap().pop_front();
    let Some(reply) = reply else {
        return Response::builder().status(599).body(Body::empty()).unwrap();
    };
    tokio::time::sleep(reply.delay).await;
    let mut response = Response::builder()
        .status(StatusCode::from_u16(reply.status).unwrap())
        .header("content-type", "application/json");
    for (name, value) in reply.headers {
        response = response.header(name, value);
    }
    response.body(Body::from(reply.body)).unwrap()
}
fn login() -> Reply {
    Reply::json(200, json!({"access_token":"fixture-token"}))
}
fn logout() -> Reply {
    Reply::raw(204, "")
}
fn job(id: &str, conversation: &str, status: &str) -> Value {
    json!({"id":id,"conversation_id":conversation,"status":status,"answer":null,"response":{"files":[]}})
}
fn artifact_job(size: u64, name: &str) -> Job {
    let mut value = job("job-1", "thread-1", "succeeded");
    value["response"]["files"] =
        json!([{"name":name,"size_bytes":size,"url":"/v1/jobs/job-1/response-files/response-1"}]);
    serde_json::from_value(value).unwrap()
}

#[tokio::test]
async fn full_flow_auth_multipart_files_followup_and_logout() {
    let mut ready = job("job-1", "thread-1", "running");
    ready["result_ready"] = json!(true);
    ready["answer"] = json!("not terminal yet");
    let mock = Mock::start(vec![
        login(),
        Reply::json(202, job("job-1", "thread-1", "queued")),
        Reply::json(200, ready),
        Reply::json(200, job("job-1", "thread-1", "succeeded")),
        Reply::json(202, job("job-2", "thread-1", "queued")),
        logout(),
    ])
    .await;
    let mut client = mock.client();
    client
        .login("fixture-user", "fixture-password")
        .await
        .unwrap();
    let temp = tempfile::tempdir().unwrap();
    let a = temp.path().join("a.txt");
    let b = temp.path().join("b.txt");
    std::fs::write(&a, "first attachment").unwrap();
    std::fs::write(&b, "second attachment").unwrap();
    let first = client
        .submit_job(JvJobRequest {
            text: "first prompt".into(),
            conversation_id: None,
            files: vec![a, b],
        })
        .await
        .unwrap();
    let result = client.wait_for_job(&first.id).await.unwrap();
    assert_eq!(result.status, JobStatus::Succeeded);
    client
        .submit_job(JvJobRequest {
            text: "next prompt".into(),
            conversation_id: Some(result.conversation_id),
            files: vec![],
        })
        .await
        .unwrap();
    client.logout().await.unwrap();
    assert!(!client.is_authenticated());
    mock.complete();
    let requests = mock.state.requests.lock().unwrap();
    let payload: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(
        payload,
        json!({"username":"fixture-user","password":"fixture-password","remember_me":false})
    );
    assert_eq!(requests[0].path, "/v1/auth/login");
    assert!(!requests[0].headers.contains_key("authorization"));
    for request in requests.iter() {
        assert_eq!(request.headers["x-jv-csrf"], "1");
    }
    for request in &requests[1..] {
        assert_eq!(request.headers["authorization"], "Bearer fixture-token");
    }
    let upload = &requests[1];
    assert_eq!(upload.method, "POST");
    assert_eq!(upload.path, "/v1/jobs");
    assert!(
        upload.headers["content-type"]
            .to_str()
            .unwrap()
            .starts_with("multipart/form-data; boundary=")
    );
    let body = String::from_utf8_lossy(&upload.body);
    assert!(body.contains("name=\"text\"\r\n\r\nfirst prompt"));
    assert!(!body.contains("conversation_id"));
    assert_eq!(body.matches("name=\"files\"").count(), 2);
    assert!(
        body.contains("filename=\"a.txt\"")
            && body.contains("first attachment")
            && body.contains("second attachment")
    );
    let followup = String::from_utf8_lossy(&requests[4].body);
    assert!(followup.contains("name=\"conversation_id\"\r\n\r\nthread-1"));
    assert!(!followup.contains("first prompt") && !followup.contains("first attachment"));
    assert!(
        requests[4].headers["content-type"]
            .to_str()
            .unwrap()
            .starts_with("multipart/form-data;")
    );
    assert_eq!(requests[5].path, "/v1/auth/logout");
    assert_eq!(requests[5].method, "POST");
}

#[tokio::test]
async fn submissions_are_never_retried_on_http_or_malformed_results() {
    for (status, body) in [
        (409, "{}"),
        (429, "{}"),
        (503, "{}"),
        (202, "not json"),
        (202, "{}"),
        (302, "{}"),
        (408, "{}"),
    ] {
        let mock = Mock::start(vec![
            login(),
            Reply::raw(status, body).header("retry-after", "17"),
            logout(),
        ])
        .await;
        let mut client = mock.client();
        client.login("u", "p").await.unwrap();
        let error = client
            .submit_job(JvJobRequest::new("question"))
            .await
            .unwrap_err();
        if matches!(status, 409 | 429) {
            assert!(
                matches!(error, Error::Http { status: s, retry_after: Some(d) } if s == status && d == Duration::from_secs(17))
            );
        } else {
            assert!(matches!(error, Error::SubmissionUncertain { .. }));
        }
        client.logout().await.unwrap();
        mock.complete();
        assert_eq!(mock.state.requests.lock().unwrap().len(), 3);
    }
}

#[tokio::test]
async fn ambiguous_network_timeout_does_not_repeat_post() {
    let mut slow = Reply::json(202, job("job-1", "thread-1", "queued"));
    slow.delay = Duration::from_millis(300);
    let mock = Mock::start(vec![login(), slow, logout()]).await;
    let mut client = JvClient::new(ClientConfig {
        base_url: mock.base.clone(),
        request_timeout: Duration::from_millis(80),
        ..ClientConfig::default()
    })
    .unwrap();
    client.login("u", "p").await.unwrap();
    assert!(matches!(
        client.submit_job(JvJobRequest::new("question")).await,
        Err(Error::SubmissionUncertain { status: None })
    ));
    client.logout().await.unwrap();
    mock.complete();
}

#[tokio::test]
async fn polling_retries_safe_transient_errors_then_returns_failure() {
    for reply in [
        Reply::raw(409, "{}"),
        Reply::raw(429, "{}").header("retry-after", "0"),
        Reply::raw(503, "{}"),
        Reply::raw(200, "not json"),
    ] {
        let mock = Mock::start(vec![
            login(),
            reply,
            Reply::json(200, job("job-1", "thread-1", "failed")),
            logout(),
        ])
        .await;
        let mut client = mock.client();
        client.login("u", "p").await.unwrap();
        assert_eq!(
            client.wait_for_job("job-1").await.unwrap().status,
            JobStatus::Failed
        );
        client.logout().await.unwrap();
        mock.complete();
    }
}

#[tokio::test]
async fn retry_after_longer_than_deadline_does_not_poll_early() {
    let mock = Mock::start(vec![
        login(),
        Reply::raw(429, "{}").header("retry-after", "3600"),
        logout(),
    ])
    .await;
    let mut client = JvClient::new(ClientConfig {
        base_url: mock.base.clone(),
        wait_timeout: Duration::from_millis(60),
        ..ClientConfig::default()
    })
    .unwrap();
    client.login("u", "p").await.unwrap();
    assert!(
        matches!(client.wait_for_job("job-1").await, Err(Error::WaitTimeout { job_id }) if job_id == "job-1")
    );
    client.logout().await.unwrap();
    mock.complete();
}

#[tokio::test]
async fn polling_deadline_bounds_inflight_request_and_retry_budget_is_finite() {
    let mut slow = Reply::raw(200, "{}");
    slow.delay = Duration::from_secs(10);
    let mock = Mock::start(vec![login(), slow, logout()]).await;
    let mut client = JvClient::new(ClientConfig {
        base_url: mock.base.clone(),
        wait_timeout: Duration::from_millis(60),
        ..ClientConfig::default()
    })
    .unwrap();
    client.login("u", "p").await.unwrap();
    assert!(matches!(
        client.wait_for_job("job-1").await,
        Err(Error::WaitTimeout { .. })
    ));
    client.logout().await.unwrap();
    mock.complete();
    let mock = Mock::start(vec![
        login(),
        Reply::raw(200, "bad"),
        Reply::raw(200, "bad"),
        Reply::raw(200, "bad"),
        logout(),
    ])
    .await;
    let mut client = mock.client();
    client.login("u", "p").await.unwrap();
    assert!(matches!(
        client.wait_for_job("job-1").await,
        Err(Error::MalformedResponse)
    ));
    client.logout().await.unwrap();
    mock.complete();
}

#[tokio::test]
async fn unauthorized_poll_fails_immediately_and_failed_logout_forgets_token() {
    let mock = Mock::start(vec![login(), Reply::raw(401, "{}"), Reply::raw(503, "{}")]).await;
    let mut client = mock.client();
    client.login("u", "p").await.unwrap();
    assert!(matches!(
        client.wait_for_job("job-1").await,
        Err(Error::Http { status: 401, .. })
    ));
    assert!(client.logout().await.is_err());
    assert!(!client.is_authenticated());
    client.logout().await.unwrap();
    assert!(matches!(
        client.get_job("job-1").await,
        Err(Error::NotAuthenticated)
    ));
    mock.complete();
}

#[tokio::test]
async fn download_is_authenticated_safe_and_does_not_overwrite_collisions() {
    let mock = Mock::start(vec![
        login(),
        Reply::raw(200, "abc"),
        Reply::raw(200, "abc"),
        logout(),
    ])
    .await;
    let mut client = mock.client();
    client.login("u", "p").await.unwrap();
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("report.txt"), "original").unwrap();
    let job = artifact_job(3, "../../report.txt");
    let first = client
        .download_response_files(&job, temp.path())
        .await
        .unwrap();
    let second = client
        .download_response_files(&job, temp.path())
        .await
        .unwrap();
    assert_eq!(first[0].file_name().unwrap(), "report-1.txt");
    assert_eq!(second[0].file_name().unwrap(), "report-2.txt");
    assert_eq!(std::fs::read(&first[0]).unwrap(), b"abc");
    assert_eq!(
        std::fs::read(temp.path().join("report.txt")).unwrap(),
        b"original"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&first[0]).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    client.logout().await.unwrap();
    mock.complete();
    let requests = mock.state.requests.lock().unwrap();
    assert_eq!(requests[1].headers["authorization"], "Bearer fixture-token");
    assert_eq!(requests[1].path, "/v1/jobs/job-1/response-files/response-1");
}

#[tokio::test]
async fn download_rejects_size_mismatch_and_redirect_without_partial_files() {
    for reply in [
        Reply::raw(200, "ab"),
        Reply::raw(200, "abcd"),
        Reply::raw(302, "").header("location", "https://evil.test/steal"),
        Reply::raw(200, "abc").header("content-encoding", "gzip"),
    ] {
        let mock = Mock::start(vec![login(), reply, logout()]).await;
        let mut client = mock.client();
        client.login("u", "p").await.unwrap();
        let temp = tempfile::tempdir().unwrap();
        assert!(
            client
                .download_response_files(&artifact_job(3, "report.txt"), temp.path())
                .await
                .is_err()
        );
        assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 0);
        client.logout().await.unwrap();
        mock.complete();
    }
}

#[tokio::test]
async fn manifest_limits_are_checked_before_any_download() {
    let mock = Mock::start(vec![login(), logout()]).await;
    let mut client = mock.client();
    client.login("u", "p").await.unwrap();
    let temp = tempfile::tempdir().unwrap();
    for size in [0, 25 * 1024 * 1024 + 1] {
        assert!(
            client
                .download_response_files(&artifact_job(size, "x"), temp.path())
                .await
                .is_err()
        );
    }
    let mut job = artifact_job(25 * 1024 * 1024, "x");
    job.response.files = vec![job.response.files[0].clone(); 5];
    assert!(
        client
            .download_response_files(&job, temp.path())
            .await
            .is_err()
    );
    job.response.files = vec![artifact_job(1, "x").response.files[0].clone(); 11];
    assert!(
        client
            .download_response_files(&job, temp.path())
            .await
            .is_err()
    );
    job = artifact_job(3, "x");
    job.response.files[0].url = "https://evil.test/steal".into();
    assert!(
        client
            .download_response_files(&job, temp.path())
            .await
            .is_err()
    );
    client.logout().await.unwrap();
    mock.complete();
}

#[cfg(unix)]
#[tokio::test]
async fn symlink_upload_destination_and_collision_are_safe() {
    use std::os::unix::fs::symlink;
    let mock = Mock::start(vec![login(), Reply::raw(200, "abc"), logout()]).await;
    let mut client = mock.client();
    client.login("u", "p").await.unwrap();
    let temp = tempfile::tempdir().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    std::fs::write(elsewhere.path().join("original"), "original").unwrap();
    symlink(elsewhere.path(), temp.path().join("linked-dir")).unwrap();
    symlink(elsewhere.path().join("original"), temp.path().join("x.txt")).unwrap();
    assert!(
        client
            .submit_job(JvJobRequest {
                text: "q".into(),
                conversation_id: None,
                files: vec![temp.path().join("x.txt")]
            })
            .await
            .is_err()
    );
    assert!(
        client
            .download_response_files(&artifact_job(3, "x.txt"), temp.path().join("linked-dir"))
            .await
            .is_err()
    );
    let paths = client
        .download_response_files(&artifact_job(3, "x.txt"), temp.path())
        .await
        .unwrap();
    assert_eq!(paths[0].file_name().unwrap(), "x-1.txt");
    assert_eq!(
        std::fs::read(elsewhere.path().join("original")).unwrap(),
        b"original"
    );
    client.logout().await.unwrap();
    mock.complete();
}

#[tokio::test]
async fn cli_json_stdout_is_one_object_and_secrets_are_not_printed() {
    for status in ["succeeded", "failed"] {
        let mut terminal = job("job-1", "thread-1", status);
        terminal["answer"] = json!("Hello ไทย");
        let mock = Mock::start(vec![
            login(),
            Reply::json(202, job("job-1", "thread-1", "queued")),
            Reply::json(200, terminal),
            logout(),
        ])
        .await;
        let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_jv-api-example"))
            .args(["question", "--json", "--base-url", &mock.base])
            .env("JV_API_USERNAME", "fixture-user")
            .env("JV_API_PASSWORD", "fixture-password")
            .output()
            .await
            .unwrap();
        assert_eq!(output.status.success(), status == "succeeded");
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(value["job_id"], "job-1");
        assert_eq!(value["conversation_id"], "thread-1");
        assert_eq!(value["status"], status);
        assert_eq!(value["answer"], "Hello ไทย");
        assert_eq!(value["files"], json!([]));
        for bytes in [&output.stdout, &output.stderr] {
            let text = String::from_utf8_lossy(bytes);
            assert!(!text.contains("fixture-password") && !text.contains("fixture-token"));
        }
        mock.complete();
    }
}

#[tokio::test]
async fn cli_reports_errors_on_stderr_and_logs_out_after_submit_failure() {
    let mock = Mock::start(vec![
        login(),
        Reply::raw(503, "private upstream detail"),
        logout(),
    ])
    .await;
    let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_jv-api-example"))
        .args(["question", "--json", "--base-url", &mock.base])
        .env("JV_API_PASSWORD", "fixture-password")
        .output()
        .await
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("uncertain"));
    assert!(!stderr.contains("private upstream detail"));
    mock.complete();
}

/// Raw HTTP exercises streaming failures that an in-memory Axum body would
/// otherwise hide behind its automatically generated Content-Length.
#[tokio::test]
async fn streamed_download_limits_disconnect_and_partial_cleanup() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    for (response, succeeds) in [
        ("HTTP/1.1 200 OK\r\nConnection: close\r\n\r\nabc", true),
        ("HTTP/1.1 200 OK\r\nConnection: close\r\n\r\nab", false),
        ("HTTP/1.1 200 OK\r\nConnection: close\r\n\r\nabcd", false),
        (
            "HTTP/1.1 200 OK\r\nContent-Length: 3\r\nConnection: close\r\n\r\nab",
            false,
        ),
    ] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let login_body = r#"{"access_token":"fixture-token"}"#;
            let replies = [
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    login_body.len(),
                    login_body
                ),
                response.to_owned(),
                "HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n".into(),
            ];
            for reply in replies {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                let mut buf = [0u8; 4096];
                loop {
                    let read = socket.read(&mut buf).await.unwrap();
                    assert!(read > 0);
                    request.extend_from_slice(&buf[..read]);
                    if let Some(end) = request.windows(4).position(|v| v == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&request[..end]);
                        let length = headers
                            .lines()
                            .find_map(|line| {
                                let (name, value) = line.split_once(':')?;
                                name.eq_ignore_ascii_case("content-length")
                                    .then(|| value.trim().parse::<usize>().unwrap())
                            })
                            .unwrap_or(0);
                        if request.len() >= end + 4 + length {
                            break;
                        }
                    }
                }
                socket.write_all(reply.as_bytes()).await.unwrap();
                socket.shutdown().await.unwrap();
            }
        });
        let mut client = JvClient::new(ClientConfig {
            base_url: base,
            ..ClientConfig::default()
        })
        .unwrap();
        client.login("u", "p").await.unwrap();
        let temp = tempfile::tempdir().unwrap();
        let result = client
            .download_response_files(&artifact_job(3, "x.txt"), temp.path())
            .await;
        assert_eq!(result.is_ok(), succeeds);
        assert_eq!(
            std::fs::read_dir(temp.path()).unwrap().count(),
            usize::from(succeeds)
        );
        client.logout().await.unwrap();
        server.await.unwrap();
    }
}

#[tokio::test]
async fn polling_recovers_from_network_timeout_without_resubmission() {
    let mut slow = Reply::json(200, job("job-1", "thread-1", "running"));
    slow.delay = Duration::from_millis(300);
    let mock = Mock::start(vec![
        login(),
        slow,
        Reply::json(200, job("job-1", "thread-1", "succeeded")),
        logout(),
    ])
    .await;
    let mut client = JvClient::new(ClientConfig {
        base_url: mock.base.clone(),
        request_timeout: Duration::from_millis(80),
        poll_interval: Duration::from_millis(5),
        wait_timeout: Duration::from_secs(2),
        ..ClientConfig::default()
    })
    .unwrap();
    client.login("u", "p").await.unwrap();
    assert_eq!(
        client.wait_for_job("job-1").await.unwrap().status,
        JobStatus::Succeeded
    );
    client.logout().await.unwrap();
    mock.complete();
    let requests = mock.state.requests.lock().unwrap();
    assert_eq!(requests[1].method, "GET");
    assert_eq!(requests[2].method, "GET");
}

#[tokio::test]
async fn invalid_login_payload_and_cross_job_response_fail_closed() {
    for body in [
        json!({}),
        json!({"access_token":""}),
        json!({"access_token":true}),
        json!({"access_token":"bad\ntoken"}),
    ] {
        let mock = Mock::start(vec![Reply::json(200, body)]).await;
        let mut client = mock.client();
        assert!(client.login("u", "p").await.is_err());
        assert!(!client.is_authenticated());
        mock.complete();
    }
    let mock = Mock::start(vec![
        login(),
        Reply::json(200, job("other", "thread-1", "succeeded")),
        logout(),
    ])
    .await;
    let mut client = mock.client();
    client.login("u", "p").await.unwrap();
    assert!(matches!(
        client.get_job("job-1").await,
        Err(Error::MalformedResponse)
    ));
    client.logout().await.unwrap();
    mock.complete();
}
