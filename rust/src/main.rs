use clap::Parser;
use jv_ai_client::{
    ClientConfig, DEFAULT_BASE_URL, Error, Job, JobStatus, JvClient, JvJobRequest, ResponseFile,
    Result,
};
use serde::Serialize;
use std::{path::PathBuf, process::ExitCode, time::Duration};
use zeroize::Zeroizing;

#[derive(Parser)]
#[command(about = "Submit one question to the JV LLM API", version)]
struct Args {
    /// Current question (only new text for a follow-up)
    question: String,
    #[arg(long, env = "JV_API_BASE_URL", default_value = DEFAULT_BASE_URL, hide_env_values = true)]
    base_url: String,
    #[arg(
        long,
        env = "JV_API_USERNAME",
        default_value = "test",
        hide_env_values = true
    )]
    username: String,
    #[arg(long)]
    conversation_id: Option<String>,
    /// Repeat for each attachment
    #[arg(long = "file")]
    files: Vec<PathBuf>,
    #[arg(long, default_value = "2", value_parser = seconds)]
    poll_interval: Duration,
    #[arg(long, default_value = "3600", value_parser = seconds)]
    wait_timeout: Duration,
    /// Print one JSON object on stdout; diagnostics go to stderr
    #[arg(long)]
    json: bool,
    #[arg(long)]
    download_dir: Option<PathBuf>,
}

fn seconds(value: &str) -> std::result::Result<Duration, String> {
    let value: f64 = value
        .parse()
        .map_err(|_| "expected a positive number of seconds")?;
    if !value.is_finite() || value <= 0.0 || value > 86400.0 * 30.0 {
        return Err("seconds must be positive, finite, and at most 30 days".into());
    }
    let duration = Duration::try_from_secs_f64(value).map_err(|_| "invalid duration")?;
    if duration.is_zero() {
        return Err("duration is too small".into());
    }
    Ok(duration)
}

#[derive(Serialize)]
struct Output<'a> {
    job_id: &'a str,
    conversation_id: &'a str,
    conversation_turn: Option<u64>,
    status: JobStatus,
    answer: Option<&'a str>,
    files: &'a [ResponseFile],
    downloaded_files: &'a [PathBuf],
    error_code: Option<&'a str>,
    error_message: Option<&'a str>,
}

async fn run_job(client: &JvClient, args: &Args) -> Result<(Job, Vec<PathBuf>)> {
    let job = client
        .submit_job(JvJobRequest {
            text: args.question.clone(),
            conversation_id: args.conversation_id.clone(),
            files: args.files.clone(),
        })
        .await?;
    eprintln!(
        "Created job {} in conversation {}.",
        job.id, job.conversation_id
    );
    let mut last = None;
    let terminal = client.wait_for_job_with_progress(&job.id, |job| {
        let state = (job.status, job.phase.clone(), job.result_ready);
        if last.as_ref() != Some(&state) {
            if job.result_ready && !job.status.is_terminal() {
                eprintln!("Answer ready; preparing chat (about one minute). Waiting for terminal status.");
            } else {
                eprintln!("Status: {:?}", job.status);
            }
            last = Some(state);
        }
    }).await?;
    let paths = if let Some(directory) = &args.download_dir {
        if terminal.status == JobStatus::Succeeded {
            client.download_response_files(&terminal, directory).await?
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };
    Ok((terminal, paths))
}

async fn run(args: &Args) -> Result<bool> {
    let mut client = JvClient::new(ClientConfig {
        base_url: args.base_url.clone(),
        poll_interval: args.poll_interval,
        wait_timeout: args.wait_timeout,
        ..ClientConfig::default()
    })?;
    let password = Zeroizing::new(match std::env::var("JV_API_PASSWORD") {
        Ok(value) => value,
        Err(std::env::VarError::NotPresent) => {
            rpassword::prompt_password("JV LLM password: ").map_err(|_| Error::FileIo)?
        }
        Err(_) => return Err(Error::InvalidInput("JV_API_PASSWORD must be valid Unicode")),
    });
    client.login(&args.username, &password).await?;
    drop(password);
    eprintln!("Authenticated.");
    let result = tokio::select! {
        result = run_job(&client, args) => result,
        _ = tokio::signal::ctrl_c() => Err(Error::Interrupted),
    };
    // All ordinary job/download failures and Ctrl-C still attempt revocation.
    let logout = client.logout().await;
    let (job, paths) = match result {
        Ok(result) => result,
        Err(error) => {
            if logout.is_err() {
                eprintln!("Warning: could not confirm token revocation.");
            }
            return Err(error);
        }
    };
    if args.json {
        let output = Output {
            job_id: &job.id,
            conversation_id: &job.conversation_id,
            conversation_turn: job.conversation_turn,
            status: job.status,
            answer: job.answer.as_deref(),
            files: &job.response.files,
            downloaded_files: &paths,
            error_code: job.error_code.as_deref(),
            error_message: job.error_message.as_deref(),
        };
        let encoded = serde_json::to_string(&output).map_err(|_| Error::MalformedResponse)?;
        println!("{encoded}");
    } else if job.status == JobStatus::Succeeded {
        println!("{}", job.answer.as_deref().unwrap_or_default());
        for path in &paths {
            eprintln!("Downloaded: {}", path.display());
        }
    } else {
        eprintln!("The JV AI job failed; use --json to inspect its public error fields.");
    }
    // A valid result remains on stdout even if revocation could not be confirmed.
    logout?;
    Ok(job.status == JobStatus::Succeeded)
}

#[tokio::main]
async fn main() -> ExitCode {
    let args = Args::parse();
    match run(&args).await {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(error) => {
            eprintln!("Error: {error}");
            if matches!(error, Error::Interrupted) {
                ExitCode::from(130)
            } else {
                ExitCode::FAILURE
            }
        }
    }
}
