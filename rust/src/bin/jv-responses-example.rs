use clap::Parser;
use jv_ai_client::{
    ClientConfig, DEFAULT_BASE_URL, Error, FunctionTool, JvClient, ResponseRequest, ResponseStatus,
    Result, ToolChoice,
};
use serde_json::json;
use std::{
    process::ExitCode,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use zeroize::Zeroizing;

static SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Parser)]
#[command(about = "Submit one text request through JV's structured Responses API pilot")]
struct Args {
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
    #[arg(long, default_value = "3", value_parser = seconds)]
    poll_interval: Duration,
    #[arg(long, default_value = "3600", value_parser = seconds)]
    wait_timeout: Duration,
    #[arg(long)]
    json: bool,
    /// Demonstrate one strictly validated tool call executed by this client
    #[arg(long)]
    tool_demo: bool,
}

fn seconds(value: &str) -> std::result::Result<Duration, String> {
    let seconds: f64 = value.parse().map_err(|_| "expected positive seconds")?;
    if !seconds.is_finite() || seconds <= 0.0 {
        return Err("seconds must be positive".into());
    }
    Duration::try_from_secs_f64(seconds).map_err(|_| "invalid duration".into())
}

fn idempotency_key() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    format!("jv-example-{nanos:x}-{:x}-{sequence:x}", std::process::id())
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
    let operation: Result<_> = async {
        let mut request = ResponseRequest::text(&args.question);
        if args.tool_demo {
            request.instructions = Some(
                "Call get_client_platform once. After its result arrives, answer briefly without requesting another tool.".into(),
            );
            request.tools = vec![FunctionTool {
                r#type: "function".into(),
                name: "get_client_platform".into(),
                description: "Return the operating-system family of this client.".into(),
                strict: true,
                parameters: json!({
                    "type": "object",
                    "properties": {},
                    "required": [],
                    "additionalProperties": false
                }),
            }];
            request.tool_choice = ToolChoice::Required;
        }
        let created = client.submit_response(&request, &idempotency_key()).await?;
        eprintln!("Created structured response {}.", created.id);
        let mut terminal = client.wait_for_response(&created.id).await?;
        if args.tool_demo {
            let (call_id, name, arguments) = terminal.function_call()?;
            if name != "get_client_platform" || serde_json::from_str::<serde_json::Value>(arguments).ok() != Some(json!({})) {
                return Err(Error::MalformedResponse);
            }
            eprintln!("Executing allowlisted local tool get_client_platform; JV Server does not execute it.");
            let continuation = ResponseRequest::continuation(&created.id, call_id, std::env::consts::OS);
            let next = client.submit_response(&continuation, &idempotency_key()).await?;
            eprintln!("Created continuation response {}.", next.id);
            terminal = client.wait_for_response(&next.id).await?;
        }
        if args.json {
            println!("{}", serde_json::to_string_pretty(&terminal).map_err(|_| Error::MalformedResponse)?);
        } else if terminal.status == ResponseStatus::Completed {
            println!("{}", terminal.output_text()?);
        } else if let Some(error) = &terminal.error {
            eprintln!("{}: {}", error.code, error.message);
        }
        Ok(terminal.status == ResponseStatus::Completed)
    }.await;
    let logout = client.logout().await;
    let succeeded = operation?;
    logout?;
    Ok(succeeded)
}

#[tokio::main]
async fn main() -> ExitCode {
    let args = Args::parse();
    match run(&args).await {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(error) => {
            eprintln!("Error: {error}");
            ExitCode::FAILURE
        }
    }
}
