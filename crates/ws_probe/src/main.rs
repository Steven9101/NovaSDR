use anyhow::Context;
use clap::Parser;
use futures_util::StreamExt;
use std::io::IsTerminal;
use std::time::Duration;
use tokio_tungstenite::tungstenite::Message;

#[derive(Debug, Parser)]
#[command(
    name = "ws_probe",
    version,
    about = "Connect to a NovaSDR WebSocket and print a short message summary."
)]
struct Args {
    /// WebSocket URL (example: ws://127.0.0.1:9002/audio)
    url: String,

    /// Number of messages to print before exiting
    #[arg(long, default_value_t = 3)]
    count: usize,

    /// Per-message read timeout (milliseconds)
    #[arg(long, default_value_t = 4000)]
    timeout_ms: u64,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_target(false)
        .with_ansi(std::io::stdout().is_terminal())
        .with_writer(std::io::stdout)
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init()
        .map_err(|e| anyhow::anyhow!("init tracing: {e}"))?;

    let args = Args::parse();
    let (mut ws, _) = tokio_tungstenite::connect_async(args.url.as_str())
        .await
        .context("connect")?;

    for idx in 0..args.count {
        let msg = tokio::time::timeout(Duration::from_millis(args.timeout_ms), ws.next())
            .await
            .context("timeout")?
            .context("websocket ended")?
            .context("read message")?;

        match msg {
            Message::Text(s) => {
                tracing::info!(idx, bytes = s.len(), "text");
            }
            Message::Binary(b) => {
                tracing::info!(idx, bytes = b.len(), "binary");
            }
            Message::Ping(b) => {
                tracing::info!(idx, bytes = b.len(), "ping");
            }
            Message::Pong(b) => {
                tracing::info!(idx, bytes = b.len(), "pong");
            }
            Message::Close(frame) => {
                tracing::info!(idx, frame = ?frame, "close");
                break;
            }
            Message::Frame(_) => {}
        }
    }

    Ok(())
}
