use autoforge::{App, Config};
use clap::{Parser, Subcommand};
use uuid::Uuid;

#[derive(Parser)]
#[command(name = "autoforge", about = "AI 외주 자동화 프로그램", version)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Actix-web API 서버 (기본)
    Serve {
        #[arg(long, env = "HOST")]
        host: Option<String>,
        #[arg(long, env = "PORT")]
        port: Option<u16>,
    },
    /// Redis Streams 커맨드 워커 (Podman 스케일아웃)
    Worker {
        #[arg(long, env = "STAGE_FILTER")]
        stage_filter: Option<String>,
        #[arg(long, env = "WORKER_ID")]
        worker_id: Option<String>,
    },
    /// 이벤트 오케스트레이터 (MQ 스케줄링)
    Orchestrate {
        #[arg(long, env = "ORCHESTRATOR_ID")]
        orchestrator_id: Option<String>,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .json()
        .init();

    let cli = Cli::parse();
    let mut config = Config::from_env();

    match cli.command.unwrap_or(Commands::Serve {
        host: None,
        port: None,
    }) {
        Commands::Serve { host, port } => {
            if let Some(h) = host {
                config.host = h;
            }
            if let Some(p) = port {
                config.port = p;
            }

            let app = if config.message_queue_enabled() {
                App::connect(config).await?.shared()
            } else {
                App::new(config).await?.shared()
            };
            autoforge::web::serve(app).await?;
        }
        Commands::Worker {
            stage_filter,
            worker_id,
        } => {
            let config = Config::from_env();
            let app = App::connect(config).await?.shared();
            let id = worker_id.unwrap_or_else(|| format!("worker-{}", Uuid::new_v4()));
            autoforge::services::pipeline::run_worker(app, id, stage_filter).await?;
        }
        Commands::Orchestrate { orchestrator_id } => {
            let config = Config::from_env();
            let app = App::connect(config).await?.shared();
            let id = orchestrator_id.unwrap_or_else(|| "orchestrator-1".into());
            autoforge::services::pipeline::run_orchestrator(app, id).await?;
        }
    }

    Ok(())
}
