//! Explicit opt-in only. Never run against a live account in ordinary CI.
use jv_ai_client::{ClientConfig, Error, JobStatus, JvClient, JvJobRequest, Result};
use zeroize::Zeroizing;

#[tokio::test]
#[ignore = "requires explicit live credentials; submits two real jobs"]
async fn live_login_upload_conversation_download_logout() -> Result<()> {
    let username = std::env::var("JV_API_USERNAME")
        .map_err(|_| Error::InvalidInput("set JV_API_USERNAME for the live test"))?;
    let password = Zeroizing::new(
        std::env::var("JV_API_PASSWORD")
            .map_err(|_| Error::InvalidInput("set JV_API_PASSWORD for the live test"))?,
    );
    let mut config = ClientConfig::default();
    if let Ok(base) = std::env::var("JV_API_BASE_URL") {
        config.base_url = base;
    }
    let mut client = JvClient::new(config)?;
    client.login(&username, &password).await?;
    drop(password);
    let result: Result<()> = async {
        let temp = tempfile::tempdir().map_err(|_| Error::FileIo)?;
        let input = temp.path().join("sample.txt");
        std::fs::write(&input, "Reference word: RUST_SMOKE_OK\n").map_err(|_| Error::FileIo)?;
        let prompt = std::env::var("JV_API_LIVE_PROMPT").unwrap_or_else(|_| {
            "Read the attached document and reply with its reference word.".into()
        });
        let created = client
            .submit_job(JvJobRequest {
                text: prompt,
                conversation_id: None,
                files: vec![input],
            })
            .await?;
        let first = client.wait_for_job(&created.id).await?;
        if first.status != JobStatus::Succeeded {
            return Err(Error::InvalidInput("first live job failed"));
        }
        let files = client
            .download_response_files(&first, temp.path().join("outputs"))
            .await?;
        if std::env::var("JV_API_LIVE_REQUIRE_FILES").as_deref() == Ok("1") && files.is_empty() {
            return Err(Error::InvalidInput(
                "live response did not contain a downloadable artifact",
            ));
        }
        let followup = client
            .submit_job(JvJobRequest {
                text: "What was the reference word in my previous document?".into(),
                conversation_id: Some(first.conversation_id.clone()),
                files: vec![],
            })
            .await?;
        let second = client.wait_for_job(&followup.id).await?;
        if second.status != JobStatus::Succeeded || second.conversation_id != first.conversation_id
        {
            return Err(Error::InvalidInput(
                "live continuation did not succeed in the same conversation",
            ));
        }
        Ok(())
    }
    .await;
    let logout = client.logout().await;
    result?;
    logout
}
