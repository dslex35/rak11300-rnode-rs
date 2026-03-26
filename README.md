# rak11300-rnode

![License: GPL-3.0](https://img.shields.io/badge/License-GPL--3.0-blue.svg)
![Target: thumbv6m-none-eabi](https://img.shields.io/badge/target-thumbv6m--none--eabi-orange.svg)

Bare-metal async Rust firmware that turns the **RAK11300** (RP2040 + SX1262) into a fully
functional **RNode** for the [Reticulum Network Stack](https://reticulum.network).  
It implements the KISS TNC-2 protocol over USB CDC-ACM, giving Reticulum a LoRa interface
it can configure and use like any other RNode.  
Fully bidirectional: received LoRa packets are forwarded to the host with RSSI/SNR reports;
KISS frames from the host are transmitted over LoRa.

---

## Hardware

| | |
|---|---|
| **Board** | RAKwireless RAK11300 |
| **MCU** | Raspberry Pi RP2040 — dual-core Cortex-M0+, 133 MHz |
| **Radio** | Semtech SX1262 — LoRa / FSK transceiver |
| **Interface** | USB CDC-ACM → `/dev/ttyACM0` (Linux) / `COMx` (Windows) |

## Confirmed RAK11300 pinout

| RP2040 GPIO | SX1262 pin | Notes |
|---|---|---|
| GPIO10 | SCLK | SPI1 clock |
| GPIO11 | MOSI | SPI1 TX |
| GPIO12 | MISO | SPI1 RX |
| GPIO13 | NSS | Chip-select, active-low |
| GPIO14 | RESET | Active-low |
| GPIO15 | BUSY | Active-high |
| GPIO29 | DIO1 | IRQ line |
| GPIO25 | RXEN | Antenna switch power enable |
| DIO2 (radio) | RF switch dir | Controlled automatically by lora-phy |
| DIO3 (radio) | TCXO supply | ~1.7 V, controlled automatically by lora-phy |

---

## Flash (quickest way)

1. Install the Rust embedded toolchain and UF2 flasher:
   ```bash
   rustup target add thumbv6m-none-eabi
   cargo install elf2uf2-rs
   ```
2. Hold the **BOOT** button on the RAK11300 while connecting USB — a drive named `RPI-RP2` appears.
3. Flash:
   ```bash
   cargo run --release
   ```
   `elf2uf2-rs` converts the ELF to UF2 and copies it to the drive automatically.  
   The board reboots into the firmware and `/dev/ttyACM0` appears.

## Build from source

```bash
rustup target add thumbv6m-none-eabi
cargo build --release
# binary: target/thumbv6m-none-eabi/release/rak11300-rnode
```

---

## Reticulum configuration

Add this block under `[interfaces]` in `~/.reticulum/config`:

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

> **Port** — `/dev/ttyACM0` on Linux, `COMx` on Windows, `/dev/cu.usbmodemXXX` on macOS.  
> **Frequency** — adjust for your region: `868200000` (EU868), `915000000` (US915), etc.

After adding the block, restart `rnsd`:

```bash
sudo systemctl restart rnsd   # or: pkill rnsd && rnsd
```

Verify the interface came up:

```bash
rnstatus | grep -A5 "RAK11300"
```

Expected output:

```
 KISSInterface[RAK11300 RNode]
    Status    : Up
    Mode      : Full
    Rate      : 1.20 kbps
    Traffic   : ↑0 B  0 bps
```

---

## KISS commands supported

| Byte | Name | Payload | Description |
|---|---|---|---|
| `0x00` | CMD_DATA | N bytes | Transmit LoRa packet; received packets forwarded to host |
| `0x01` | CMD_FREQ | 4 bytes BE (Hz) | Set radio frequency |
| `0x02` | CMD_BW | 4 bytes BE (Hz) | Set bandwidth |
| `0x03` | CMD_TXPWR | 1 byte (dBm) | Set TX power |
| `0x04` | CMD_SF | 1 byte (5–12) | Set spreading factor |
| `0x05` | CMD_CR | 1 byte (5–8) | Set coding rate (4/5 … 4/8) |
| `0x23` | CMD_STAT_RSSI | 1 byte | RSSI report after RX — value = `−rssi_dBm` |
| `0x24` | CMD_STAT_SNR | 1 byte | SNR report after RX — value = `(snr_dB + 32) × 4` |

---

## Key dependencies

- [embassy-rp](https://github.com/embassy-rs/embassy) — RP2040 async HAL (SPI, USB, GPIO, DMA)
- [embassy-executor](https://github.com/embassy-rs/embassy) — cooperative async executor (Cortex-M)
- [embassy-usb](https://github.com/embassy-rs/embassy) — USB device stack with CDC-ACM class
- [embassy-futures](https://github.com/embassy-rs/embassy) — `select()` for concurrent RX + TX
- [lora-phy v3](https://github.com/lora-rs/lora-rs) — async SX1262 driver
- [embedded-hal-bus](https://github.com/rust-embedded/embedded-hal) — `ExclusiveDevice` SPI wrapper
- [heapless](https://github.com/japaric/heapless) — stack-allocated KISS frame buffer
- [defmt](https://github.com/knurling-rs/defmt) + defmt-rtt — structured RTT logging

---

## Project layout

```
src/main.rs        — firmware: KISS decoder/encoder, LoRa RX/TX, USB CDC, concurrent select()
Cargo.toml         — dependencies and build profiles (opt-level = s, LTO)
.cargo/config.toml — target thumbv6m-none-eabi, probe-rs / elf2uf2-rs runner, linker flags
memory.x           — RP2040 memory layout: BOOT2 (256 B), FLASH (2 MB), RAM (256 KB)
build.rs           — adds project root to linker search path so memory.x is found
```

---

## License

[GPL-3.0](LICENSE) — Made for the Reticulum community. Pull requests welcome.