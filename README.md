# rak11300-rnode

> Rust RNode firmware for RAK11300 (RP2040 + SX1262)

[![License: GPL-3.0](https://img.shields.io/badge/License-GPL--3.0-blue.svg)](LICENSE)
[![Target: thumbv6m-none-eabi](https://img.shields.io/badge/target-thumbv6m--none--eabi-lightgrey.svg)]()

## What it does

Bare-metal async Rust firmware using [Embassy](https://embassy.dev) that turns the RAK11300 into an RNode — a KISS-over-USB-CDC LoRa modem compatible with the [Reticulum Network Stack](https://reticulum.network). Receives LoRa packets and forwards them to the host; receives KISS frames from the host and transmits over LoRa.

## Hardware

| Component | Detail |
|-----------|--------|
| Board | RAKwireless RAK11300 |
| MCU | RP2040 (dual-core Cortex-M0+, 133 MHz) |
| Radio | Semtech SX1262 (LoRa / FSK) |
| Interface | USB CDC-ACM (`/dev/ttyACM0`) |

## Confirmed RAK11300 Pinout

| GPIO | SX1262 Pin | Notes |
|------|------------|-------|
| GPIO10 | SCK | SPI clock |
| GPIO11 | MOSI | SPI data out |
| GPIO12 | MISO | SPI data in |
| GPIO13 | NSS | SPI chip select |
| GPIO14 | RESET | Radio reset |
| GPIO15 | BUSY | Radio busy signal |
| GPIO29 | DIO1 | IRQ line |
| GPIO25 | RXEN | Antenna switch power |
| — | DIO2 | Antenna switch direction (auto via lora-phy) |
| — | DIO3 | TCXO supply ~1.7 V (auto via lora-phy) |

## Flash (quickest way)

1. Install Rust and the RP2040 target:
   ```bash
   rustup target add thumbv6m-none-eabi
   cargo install elf2uf2-rs
   ```
2. Hold the **BOOT** button while plugging in USB — the board mounts as `RPI-RP2`.
3. Flash:
   ```bash
   cargo run --release
   ```

## Build from Source

```bash
rustup target add thumbv6m-none-eabi
cargo install elf2uf2-rs
cargo build --release
```

## Reticulum Configuration

Add the following block under `[interfaces]` in `~/.reticulum/config`:

```toml
[[RAK11300 RNode]]
type = KISSInterface
enabled = Yes
port = /dev/ttyACM0
speed = 115200
frequency = 868200000
bandwidth = 125000
txpower = 14
spreadingfactor = 7
codingrate = 5
```

The port is `/dev/ttyACM0` on Linux or `COM3` (or similar) on Windows. After saving, restart `rnsd`.

Verify the interface is up:

```bash
rnstatus | grep -A5 "RAK11300"
```

Expected output includes `Status : Up` and incrementing traffic counters once packets flow.

## KISS Commands Supported

| Command | Byte | Description |
|---------|------|-------------|
| CMD_DATA | `0x00` | Send / receive a LoRa packet |
| CMD_FREQ | `0x01` | Set centre frequency (Hz) |
| CMD_BW | `0x02` | Set bandwidth (Hz) |
| CMD_TXPWR | `0x03` | Set TX power (dBm) |
| CMD_SF | `0x04` | Set spreading factor |
| CMD_CR | `0x05` | Set coding rate |
| CMD_STAT_RSSI | `0x23` | Last packet RSSI |
| CMD_STAT_SNR | `0x24` | Last packet SNR |

## Key Dependencies

- [`embassy-rp`](https://crates.io/crates/embassy-rp) — RP2040 HAL + peripherals
- [`embassy-executor`](https://crates.io/crates/embassy-executor) — async task executor
- [`embassy-usb`](https://crates.io/crates/embassy-usb) — USB device stack (CDC-ACM)
- [`lora-phy`](https://crates.io/crates/lora-phy) v3 — SX1262 driver
- [`embassy-futures`](https://crates.io/crates/embassy-futures) — async utilities
- [`embedded-hal-bus`](https://crates.io/crates/embedded-hal-bus) — SPI bus sharing
- [`heapless`](https://crates.io/crates/heapless) — fixed-size collections
- [`defmt`](https://crates.io/crates/defmt) — lightweight logging

## Project Layout

```
src/main.rs          — full firmware (KISS decoder, LoRa RX/TX, USB CDC)
Cargo.toml           — dependencies
.cargo/config.toml   — target thumbv6m-none-eabi + linker flags
memory.x             — RP2040 memory layout
build.rs             — linker search path
```

## License

[GPL-3.0](LICENSE)