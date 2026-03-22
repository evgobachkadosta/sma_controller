mod config;
mod controller;
mod csv_parser;
mod inverter;

use anyhow::{Context, Result};
use std::path::PathBuf;
use tracing::info;

/// On SIGTERM or Ctrl-C, it restores full power and logs out cleanly.
#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let config_path = parse_args(&args)?;

    let config = config::Config::load(&config_path)
        .with_context(|| format!("Cannot load config from {}", config_path.display()))?;

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_target(false)
        .init();

    info!("sma-controller starting up");
    info!("Config loaded from: {}", config_path.display());
    info!(
        "Inverter: {}:{} (max {} W)",
        config.inverter.host, config.inverter.port, config.inverter.max_power_watts
    );
    info!(
        "Threshold: {} EUR/MWh | Limit: {} W | CSV dir: {}",
        config.control.price_threshold_eur_mwh,
        config.control.limit_power_watts,
        config.control.csv_dir.display()
    );

    let mut controller = controller::Controller::new(config)?;

    let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel::<()>();

    tokio::spawn(async move {
        #[cfg(unix)]
        {
            use tokio::signal::unix::{signal, SignalKind};
            let mut sigterm = signal(SignalKind::terminate())
                .expect("Failed to register SIGTERM handler");
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {},
                _ = sigterm.recv() => {},
            }
        }
        #[cfg(not(unix))]
        {
            let _ = tokio::signal::ctrl_c().await;
        }
        info!("Shutdown signal received");
        let _ = shutdown_tx.send(());
    });

    tokio::select! {
        result = controller.run() => {
            if let Err(e) = result {
                tracing::error!("Controller exited with error: {}", e);
                controller.shutdown().await;
                return Err(e);
            }
        }
        _ = &mut shutdown_rx => {
            controller.shutdown().await;
        }
    }

    info!("sma-controller exited cleanly");
    Ok(())
}

fn parse_args(args: &[String]) -> Result<PathBuf> {
    let mut config_path: Option<PathBuf> = None;
    let mut iter = args.iter().skip(1);

    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--config" | "-c" => {
                let val = iter
                    .next()
                    .context("--config requires a path argument")?;
                config_path = Some(PathBuf::from(val));
            }
            "--help" | "-h" => {
                print_usage(&args[0]);
                std::process::exit(0);
            }
            other => {
                anyhow::bail!("Unknown argument: {}. Use --help for usage.", other);
            }
        }
    }


    config_path.context(
        "Missing required argument: --config <path>\nUse --help for usage.",
    )
}

fn print_usage(prog: &str) {
    eprintln!(
        r#"Usage: {prog} --config <path>

Options:
  -c, --config <path>   Path to the TOML configuration file (required)
  -h, --help            Print this help message

Environment:
  RUST_LOG              Log level filter (default: info)
                        Examples: debug, warn, sma_controller=trace

The process runs indefinitely, polling the inverter every N seconds.
Launch it from cron at midnight with the new daily price CSV already in place:

  0 0 * * * /usr/local/bin/sma-controller --config /etc/sma-controller/config.toml

Send SIGTERM or press Ctrl-C to stop gracefully (restores full power first).
"#,
        prog = prog
    );
}
