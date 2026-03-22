use anyhow::{Context, Result, bail};
use chrono::NaiveTime;
use std::path::Path;


#[derive(Debug, Clone)]
pub struct PriceSlot {
    pub start: NaiveTime,
    pub end: NaiveTime,
    pub price_eur_mwh: f64,
}
pub fn parse_price_csv(path: &Path) -> Result<Vec<PriceSlot>> {
    let contents = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read CSV file: {}", path.display()))?;

    let mut slots: Vec<PriceSlot> = Vec::with_capacity(96);

    for (line_no, line) in contents.lines().enumerate() {
        let line = line.trim();

        if line.is_empty() {
            continue;
        }

        if !line.starts_with("QH") {
            tracing::debug!("Skipping non-QH line {}: {:?}", line_no + 1, line);
            continue;
        }

        let slot = parse_line(line)
            .with_context(|| format!("Failed to parse CSV line {}: {:?}", line_no + 1, line))?;

        slots.push(slot);
    }

    if slots.is_empty() {
        bail!("No price slots found in CSV file: {}", path.display());
    }

    if slots.len() != 96 {
        tracing::warn!(
            "Expected 96 quarter-hour slots in CSV, found {}. Proceeding anyway.",
            slots.len()
        );
    }

    tracing::info!(
        "Loaded {} price slots from {}",
        slots.len(),
        path.display()
    );

    Ok(slots)

}
fn parse_line(line: &str) -> Result<PriceSlot> {
    let mut parts = line.split(';');

    let _label = parts.next().context("Missing QH label column")?;
    let time_range = parts.next().context("Missing time range column")?.trim();
    let price_str = parts.next().context("Missing price column")?.trim();

    let (start, end) = parse_time_range(time_range)
        .with_context(|| format!("Invalid time range: {:?}", time_range))?;

    let price_eur_mwh: f64 = price_str
        .parse()
        .with_context(|| format!("Invalid price value: {:?}", price_str))?;

    Ok(PriceSlot { start, end, price_eur_mwh })
}

fn parse_time_range(range: &str) -> Result<(NaiveTime, NaiveTime)> {
    let (start_str, end_str) = range
        .split_once(" - ")
        .with_context(|| format!("Expected 'HH:MM - HH:MM' format, got: {:?}", range))?;

    let start = parse_hhmm(start_str.trim())
        .with_context(|| format!("Invalid start time: {:?}", start_str))?;
    let raw_end = parse_hhmm(end_str.trim())
        .with_context(|| format!("Invalid end time: {:?}", end_str))?;

    let end = if raw_end == NaiveTime::from_hms_opt(0, 0, 0).unwrap()
        && start >= NaiveTime::from_hms_opt(23, 0, 0).unwrap()
    {
        // Treat as end-of-day
        NaiveTime::from_hms_opt(23, 59, 59).unwrap()
    } else {
        raw_end
    };

    Ok((start, end))
}

fn parse_hhmm(s: &str) -> Result<NaiveTime> {
    NaiveTime::parse_from_str(s, "%H:%M")
        .with_context(|| format!("Cannot parse time {:?} as HH:MM", s))
}

pub fn slot_for_time(slots: &[PriceSlot], time: NaiveTime) -> Option<&PriceSlot> {
    slots.iter().find(|s| time >= s.start && time < s.end)
}

