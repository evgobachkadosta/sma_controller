# sma-controller

Automatic power limiter for the **SMA 180-21** inverter.

Reads a daily electricity spot-price CSV (quarter-hour resolution) and lowers
the inverter's active power output whenever the current slot price falls below
a configurable threshold. Restores full power automatically when the price
rises back above the threshold.

---

## How it works

1. At startup, the app logs in to the inverter's WebConnect API and loads
   today's price CSV from the configured directory.
2. Every 60 seconds (configurable) it finds the current 15-minute price slot
   and compares the price to your threshold.
3. **Price < threshold** → sends `setParamValues` with `limit_power_watts`
4. **Price ≥ threshold** → sends `setParamValues` with `max_power_watts` (full power)
5. Commands are only sent when the desired state **changes** — no hammering the inverter.
6. After every write, the app waits 5 seconds and reads the value back to confirm
   the inverter accepted the change. If it didn't, it re-logs in and retries once.
7. At midnight the CSV for the new day is loaded automatically.
8. On `SIGTERM` or `Ctrl-C` full power is restored before exit.

---

## Build

### Requirements

- Rust 1.75 or newer
  Install via [rustup](https://rustup.rs):
  ```bash
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
  source $HOME/.cargo/env
  ```

### Compile (release build)

```bash
cargo build --release
```

The binary is at `target/release/sma-controller`.

---

## Configuration

Edit `config.toml`:

```toml
[inverter]
host = "192.168.1.100"       # inverter IP on your LAN
port = 18443                 # default SMA WebConnect port
right = "istl"               # login role (installer)
password = "your_password"
max_power_watts = 180000     # 180 kW rated output

[control]
csv_dir = "/tmp/day_ahead_price"             # directory where daily CSVs are generated
price_threshold_eur_mwh = 80.0  # limit when price < 80 EUR/MWh
limit_power_watts = 500          # reduce to 500 W when limiting
poll_interval_secs = 60
```

---

## Running

### Manual (foreground, useful for testing)

```bash
sma-controller --config /etc/sma-controller/config.toml
```

Control log verbosity with `RUST_LOG`:

```bash
RUST_LOG=debug sma-controller --config /etc/sma-controller/config.toml
```

## Troubleshooting

| Symptom | Likely cause |
|---|---|
| `Failed to read config file` | Wrong path to config.toml |
| `Failed to read CSV file` | Today's `dam_data_{date}.csv` not yet generated in `csv_dir` |
| `Initial login failed` | Wrong IP, port, password, or inverter unreachable |
| `Inverter returned setParamValues error` | Session expired (app re-logins automatically) or wrong parameter key |
| `Verification failed: sent X W, inverter reports Y W` | Value out of range or inverter in a state that prevents changes |
| `No price slot found for current time` | CSV missing or wrong format |
| `Expected 96 quarter-hour slots, found N` | Partial or malformed CSV |

---
  
