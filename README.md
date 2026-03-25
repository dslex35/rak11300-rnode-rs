# rak11300-rnode-rs

**Rust native RNode firmware for the RAK11300 (RP2040 + SX1262)**

A lightweight, bare-metal implementation that turns your RAK11300 module into a fully functional **RNode** for the [Reticulum](https://reticulum.network) network stack.

## What is this?

- Runs directly on the RP2040 (no OS)
- Uses the onboard SX1262 LoRa radio
- Implements RNode protocol over LoRa
- Compatible with Reticulum (Python, Rust, or any other RNS implementation)

## Features

- Small binary (UF2 provided)
- Low resource usage
- Standard RNode KISS-like interface over Reticulum
- Easy to flash and use

## Quick Start

### 1. Flash the firmware

1. Put your RAK11300 (or board with RAK11300) into **bootloader mode** (hold BOOT button while powering on / resetting).
2. Copy the provided `rak11300-rnode.uf2` file to the mounted `RPI-RP2` drive.
3. The board will reboot automatically with the RNode firmware.

### 2. Use with Reticulum

Once flashed, the device appears as a normal **RNode** interface.

On your computer (or another device running Reticulum):

```bash
# Install Reticulum if not already installed
pip install rns

# Discover and configure the RNode (usually over USB serial)
rnodeconf /dev/ttyACM0 --autoinstall   # adjust port as needed
```
# Or add it manually in your Reticulum config
The RNode will now participate in your Reticulum mesh network over LoRa.
Project Structure

src/main.rs — main firmware (bare-metal Rust + embassy / rp2040-hal)
Cargo.toml — Rust dependencies and build config
memory.x / build.rs — linker script and build helpers for RP2040
rak11300-rnode.uf2 — pre-built binary

Building from source (optional)
Bashcargo build --release
# The UF2 will be generated in target/...
License
GPL-3.0

Made for the Reticulum community.
Pull requests and issues welcome!
