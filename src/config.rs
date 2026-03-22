use anyhow::{Context, Result};
use chrono::Local;
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    pub inverter: InverterConfig,
    pub control: ControlConfig,
}

#[derive(Debug, Deserialize, Clone)]
pub struct InverterConfig {
    pub host: String,

    #[serde(default = "default_port")]
    pub port: u16,

    pub right: String,

    pub password: String,

    /// Maximum rated output of the inverter in watts (used when restoring full power)
    /// For the SMA 180-21 this is 180000 W (180 kW)
    pub max_power_watts: u32,
}

fn default_port() -> u16 {
    18443
}

#[derive(Debug, Deserialize, Clone)]
pub struct ControlConfig {
    pub csv_dir: PathBuf,

    pub price_threshold_eur_mwh: f64,

    pub limit_power_watts: u32,

    #[serde(default = "default_poll_interval")]
    pub poll_interval_secs: u64,
}

fn default_poll_interval() -> u64 {
    60
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let contents = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read config file: {}", path.display()))?;

        let config: Config = toml::from_str(&contents)
            .with_context(|| format!("Failed to parse config file: {}", path.display()))?;

        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<()> {
        anyhow::ensure!(
            !self.inverter.host.is_empty(),
            "inverter.host must not be empty"
        );
        anyhow::ensure!(
            self.inverter.max_power_watts > 0,
            "inverter.max_power_watts must be > 0"
        );
        anyhow::ensure!(
            self.control.limit_power_watts > 0,
            "control.limit_power_watts must be > 0"
        );
        anyhow::ensure!(
            self.control.limit_power_watts < self.inverter.max_power_watts,
            "control.limit_power_watts ({}) must be less than inverter.max_power_watts ({})",
            self.control.limit_power_watts,
            self.inverter.max_power_watts
        );
        anyhow::ensure!(
            self.control.price_threshold_eur_mwh > 0.0,
            "control.price_threshold_eur_mwh must be > 0"
        );
        anyhow::ensure!(
            self.control.poll_interval_secs > 0,
            "control.poll_interval_secs must be > 0"
        );
        Ok(())
    }

    pub fn inverter_base_url(&self) -> String {
        format!("https://{}:{}", self.inverter.host, self.inverter.port)
    }
}

impl ControlConfig {
    pub fn csv_path_for_today(&self) -> PathBuf {
        let today = Local::now().format("%Y-%m-%d");
        self.csv_dir.join(format!("dam_data_{}.csv", today))
    }
}
