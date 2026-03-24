mod config;
mod controller;
mod csv_parser;
mod inverter;

use anyhow::{Context, Result};
use std::path::PathBuf;
use tracing::info;

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

    info!("sma-controller starting");
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

    controller::run_once(config).await
}


fn parse_args(args: &[String]) -> Result<PathBuf> {
    let mut config_path: Option<PathBuf> = None;
    let mut iter = args.iter().skip(1); // skip argv[0]

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
                        Examples: debug, warn

Intended to be run by cron every 10 minutes:

  */10 * * * * /usr/local/bin/sma-controller --config /etc/sma-controller/config.toml
"#,
        prog = prog
    );
}
