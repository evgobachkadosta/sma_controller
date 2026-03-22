use anyhow::{Context, Result};
use chrono::Local;
use tracing::{error, info, warn};

use crate::{
    config::Config,
    csv_parser::{parse_price_csv, slot_for_time, PriceSlot},
    inverter::InverterClient,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PowerState {
    Unknown,
    Limited,
    Full,
}

pub struct Controller {
    config: Config,
    client: InverterClient,
}

impl Controller {
    pub fn new(config: Config) -> Result<Self> {
        let client = InverterClient::new(config.clone())?;
        Ok(Self { config, client })
    }

    pub async fn run(&mut self) -> Result<()> {
        self.client
            .login()
            .await
            .context("Initial login failed")?;

        let mut slots = self.load_csv()?;
        let mut current_date = Local::now().date_naive();
        let mut last_power_state = PowerState::Unknown;

        let interval = std::time::Duration::from_secs(self.config.control.poll_interval_secs);

        info!(
            "Controller started. Poll interval: {}s, threshold: {} EUR/MWh, \
             limit: {} W, max: {} W, csv dir: {}",
            self.config.control.poll_interval_secs,
            self.config.control.price_threshold_eur_mwh,
            self.config.control.limit_power_watts,
            self.config.inverter.max_power_watts,
            self.config.control.csv_dir.display(),
        );

        loop {
            let now = Local::now();
            let today = now.date_naive();

            if today != current_date {
                info!("New date available ({}), reloading CSV", today);
                match self.load_csv() {
                    Ok(new_slots) => {
                        slots = new_slots;
                        current_date = today;
                        last_power_state = PowerState::Unknown;
                        info!("CSV reloaded successfully for {}", today);
                    }
                    Err(e) => {
                        error!(
                            "Failed to reload CSV for {}: {}. \
                             Continuing with yesterday's data.",
                            today, e
                        );
                    }
                }
            }

            let current_time = now.time();
            let desired_state = self.evaluate_slot(&slots, current_time);

            if desired_state != last_power_state {
                match self.apply_state(desired_state).await {
                    Ok(()) => {
                        last_power_state = desired_state;
                    }
                    Err(e) => {
                        error!("Failed to apply power state {:?}: {}", desired_state, e);
                    }
                }
            } else {
                info!(
                    "Time: {}, state unchanged ({:?}), no command sent",
                    current_time.format("%H:%M:%S"),
                    last_power_state
                );
            }

            tokio::time::sleep(interval).await;
        }
    }

    fn evaluate_slot(&self, slots: &[PriceSlot], time: chrono::NaiveTime) -> PowerState {
        match slot_for_time(slots, time) {
            None => {
                warn!(
                    "No price slot found for current time {}. \
                     Defaulting to full power (safe fallback).",
                    time.format("%H:%M:%S")
                );
                PowerState::Full
            }
            Some(slot) => {
                let threshold = self.config.control.price_threshold_eur_mwh;
                let price = slot.price_eur_mwh;

                if price < threshold {
                    info!(
                        "Slot {}–{}: price {:.2} EUR/MWh < threshold {:.2} → LIMIT to {} W",
                        slot.start.format("%H:%M"),
                        slot.end.format("%H:%M"),
                        price,
                        threshold,
                        self.config.control.limit_power_watts
                    );
                    PowerState::Limited
                } else {
                    info!(
                        "Slot {}–{}: price {:.2} EUR/MWh >= threshold {:.2} → FULL power ({} W)",
                        slot.start.format("%H:%M"),
                        slot.end.format("%H:%M"),
                        price,
                        threshold,
                        self.config.inverter.max_power_watts
                    );
                    PowerState::Full
                }
            }
        }
    }

    async fn apply_state(&mut self, state: PowerState) -> Result<()> {
        let watts = match state {
            PowerState::Limited => self.config.control.limit_power_watts,
            PowerState::Full => self.config.inverter.max_power_watts,
            PowerState::Unknown => unreachable!("Unknown state should never be applied"),
        };
        info!("Sending set_power_watts({})", watts);
        self.client.set_power_watts(watts).await
    }

    fn load_csv(&self) -> Result<Vec<PriceSlot>> {
        let path = self.config.control.csv_path_for_today();
        info!("Loading CSV: {}", path.display());
        parse_price_csv(&path)
    }

    pub async fn shutdown(&mut self) {
        info!("Shutdown — restoring full power before exit");
        let max = self.config.inverter.max_power_watts;
        if let Err(e) = self.client.set_power_watts(max).await {
            error!("Failed to restore full power on shutdown: {}", e);
        } else {
            info!("Full power restored ({}W)", max);
        }
        self.client.logout().await;
    }
}
