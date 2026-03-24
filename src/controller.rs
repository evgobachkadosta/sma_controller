use anyhow::{Context, Result};
use chrono::Local;
use tracing::info;

use crate::{
    config::Config,
    csv_parser::{parse_price_csv, slot_for_time},
    inverter::InverterClient,
};

pub async fn run_once(config: Config) -> Result<()> {
    let mut client = InverterClient::new(config.clone())?;
    client.login().await.context("Login failed")?;

    let desired_watts = {
        let csv_path = config.control.csv_path_for_today();
        let slots = parse_price_csv(&csv_path)?;
        let now = Local::now().time();

        let slot = slot_for_time(&slots, now).with_context(|| {
            format!("No price slot found for current time {}", now.format("%H:%M"))
        })?;

        let threshold = config.control.price_threshold_eur_mwh;

        if slot.price_eur_mwh < threshold {
            info!(
                "Slot {}–{}: price {:.2} EUR/MWh < threshold {:.2} → LIMIT to {} W",
                slot.start.format("%H:%M"),
                slot.end.format("%H:%M"),
                slot.price_eur_mwh,
                threshold,
                config.control.limit_power_watts,
            );
            config.control.limit_power_watts
        } else {
            info!(
                "Slot {}–{}: price {:.2} EUR/MWh >= threshold {:.2} → FULL power ({} W)",
                slot.start.format("%H:%M"),
                slot.end.format("%H:%M"),
                slot.price_eur_mwh,
                threshold,
                config.inverter.max_power_watts,
            );
            config.inverter.max_power_watts
        }
    };

    let current_watts = client
        .get_power_limit_watts()
        .await
        .context("Failed to read current power limit from inverter")?;

    info!("Inverter current power limit: {} W", current_watts);

    if current_watts == desired_watts {
        info!(
            "Inverter already at desired value ({} W), no write needed",
            desired_watts
        );
        client.logout().await;
        return Ok(());
    }
 
    info!(
        "Changing power limit: {} W → {} W",
        current_watts, desired_watts
    );
    client
        .set_power_watts(desired_watts)
        .await
        .context("Failed to set power limit")?;

    client.logout().await;

    Ok(())
}
