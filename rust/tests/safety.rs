use jv_ai_client::{
    ClientConfig, Error, Job, JobStatus, JvClient, JvJobRequest, ResponseFile, parse_retry_after,
    safe_filename, validate_base_url, validate_download_url,
};
use serde_json::json;
use std::time::{Duration, SystemTime};

#[test]
fn accepts_only_safe_api_origins() {
    for value in [
        "https://ai.openjvspace.com",
        "https://example.com:8443/",
        "http://127.0.0.1:61669",
        "http://localhost:61669",
    ] {
        assert!(validate_base_url(value).is_ok());
    }
    for value in [
        "http://example.com",
        "file:///tmp/file",
        "https://user:password@example.com",
        "https://@example.com",
        "https://example.com/path",
        "https://example.com/a/..",
        "https://example.com/?token=x",
        "https://example.com/#x",
        "https://example.com\\evil",
        "https://exam\nple.com",
    ] {
        assert!(validate_base_url(value).is_err(), "accepted unsafe origin");
    }
}

#[test]
fn response_urls_cannot_escape_origin_or_job() {
    let base = validate_base_url("https://example.com").unwrap();
    assert_eq!(
        validate_download_url(&base, "job-1", "/v1/jobs/job-1/response-files/response-1")
            .unwrap()
            .as_str(),
        "https://example.com/v1/jobs/job-1/response-files/response-1"
    );
    for value in [
        "https://evil.test/file",
        "https://example.com/v1/jobs/job-1/response-files/response-1",
        "//evil.test/file",
        "/v1/jobs/other/response-files/response-1",
        "/v1/jobs/job-1/response-files/../other",
        "/v1/jobs/job-1/response-files/%2e%2e",
        "/v1/jobs/job-1/response-files/%252f",
        "/v1/jobs/job-1/response-files/a/b",
        "/v1/jobs/job-1/response-files/a\\b",
        "/v1/jobs/job-1/response-files/a?token=x",
        "/v1/jobs/job-1/response-files/a#x",
        "/v1/jobs/job-1/response-files/",
    ] {
        assert!(validate_download_url(&base, "job-1", value).is_err());
    }
    assert!(
        validate_download_url(&base, "../escape", "/v1/jobs/../escape/response-files/a").is_err()
    );
}

#[test]
fn filenames_are_portable_single_components() {
    assert_eq!(safe_filename("../../report.txt", 1), "report.txt");
    assert_eq!(safe_filename(r"C:\private\report.txt", 1), "report.txt");
    assert_eq!(safe_filename(" .. ", 3), "jv-ai-output-3");
    assert_eq!(safe_filename("CON.txt", 1), "_CON.txt");
    assert_eq!(safe_filename("LPT9", 1), "_LPT9");
    assert_eq!(safe_filename("report:stream.txt", 1), "report_stream.txt");
    for value in [
        "\0../../",
        "..",
        "...",
        "aux",
        "NUL.txt",
        "ไทย.txt",
        "/etc/passwd",
        "a\nb.txt",
    ] {
        let name = safe_filename(value, 1);
        assert!(!name.is_empty() && !name.contains(['/', '\\', ':', '\0', '\n']));
        assert!(!name.starts_with('.') && !name.ends_with([' ', '.']));
    }
    assert_eq!(safe_filename(&"x".repeat(500), 1).len(), 180);
}

#[test]
fn retry_after_supports_seconds_and_http_dates() {
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
    assert_eq!(
        parse_retry_after(" 42 ", now),
        Some(Duration::from_secs(42))
    );
    assert_eq!(parse_retry_after("0", now), Some(Duration::ZERO));
    assert_eq!(
        parse_retry_after(&httpdate::fmt_http_date(now + Duration::from_secs(17)), now),
        Some(Duration::from_secs(17))
    );
    assert_eq!(
        parse_retry_after(&httpdate::fmt_http_date(now - Duration::from_secs(17)), now),
        Some(Duration::ZERO)
    );
    for value in [
        "",
        "-1",
        "+1",
        "1.2",
        "tomorrow",
        "999999999999999999999999999",
    ] {
        assert_eq!(parse_retry_after(value, now), None);
    }
}

#[test]
fn statuses_match_central_contract_and_unknown_is_rejected() {
    for value in [
        "queued",
        "dispatching",
        "waiting_for_provider",
        "running",
        "waiting_for_auth",
    ] {
        let status: JobStatus = serde_json::from_value(json!(value)).unwrap();
        assert!(!status.is_terminal());
    }
    for value in ["succeeded", "failed"] {
        assert!(
            serde_json::from_value::<JobStatus>(json!(value))
                .unwrap()
                .is_terminal()
        );
    }
    for value in ["timeout", "result_ready", "unknown", "", "SUCCESS"] {
        assert!(serde_json::from_value::<JobStatus>(json!(value)).is_err());
    }
}

#[test]
fn public_job_deserializes_nullable_fields_and_response_files() {
    let job: Job = serde_json::from_value(json!({
        "id":"job-1", "conversation_id":"thread-1", "conversation_turn":2,
        "status":"running", "result_ready":true, "answer":"Ready", "phase":"cleanup",
        "queue_position":null, "error_code":null, "error_message":null,
        "response":{"text":"Ready", "files":[{"name":"a.txt", "url":"/v1/jobs/job-1/response-files/response-1", "size_bytes":3}]},
        "unknown_future_field":"ignored"
    })).unwrap();
    assert_eq!(job.conversation_id, "thread-1");
    assert!(!job.status.is_terminal());
    assert_eq!(job.response.files[0].size_bytes, 3);
    assert!(serde_json::from_value::<Job>(json!({"id":"a","status":"succeeded"})).is_err());
}

#[test]
fn malformed_file_sizes_are_rejected_by_deserialization() {
    for size in [json!(true), json!(-1), json!(1.5), json!("10"), json!(null)] {
        assert!(
            serde_json::from_value::<ResponseFile>(
                json!({"name":"x","url":"/x","size_bytes":size})
            )
            .is_err()
        );
    }
}

#[test]
fn conversation_requests_preserve_exact_owned_id() {
    let mut request = JvJobRequest::new("new question");
    assert!(request.validate().is_ok());
    assert!(request.conversation_id.is_none());
    request.conversation_id = Some("thread-123".into());
    assert!(request.validate().is_ok());
    assert_eq!(request.conversation_id.as_deref(), Some("thread-123"));
    for id in ["", " ", "../a", "a/b", "a?x=1", " a "] {
        request.conversation_id = Some(id.into());
        assert!(request.validate().is_err());
    }
    assert!(JvJobRequest::new("  ").validate().is_err());
}

#[test]
fn rejects_invalid_timeouts_without_network() {
    let config = ClientConfig {
        poll_interval: Duration::ZERO,
        ..ClientConfig::default()
    };
    assert!(matches!(JvClient::new(config), Err(Error::InvalidInput(_))));
}
