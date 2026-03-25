#![no_std]
#![no_main]

// ============================================================
//  RAK11300 RNode — KISS-over-USB-CDC ↔ SX1262 LoRa bridge
//
//  Hardware (confirmed from RAK11300 documentation):
//    SPI1  SCK  → GPIO10   SX1262 SCLK
//    SPI1  MOSI → GPIO11   SX1262 MOSI
//    SPI1  MISO → GPIO12   SX1262 MISO
//    GPIO13              → SX1262 NSS  (chip-select, active-low)
//    GPIO14              → SX1262 RESET (active-low)
//    GPIO15              → SX1262 BUSY  (active-high)
//    GPIO29              → SX1262 DIO1  (IRQ)
//    GPIO25              → Antenna-switch power enable (RXEN)
//    DIO2  (SX1262 pin)  → Antenna-switch TX/RX direction (automatic)
//    DIO3  (SX1262 pin)  → TCXO supply voltage control  (automatic)
// ============================================================

use defmt::{error, info, unwrap, warn};
use embassy_executor::Spawner;
use embassy_futures::select::{select, Either};
use embassy_rp::{
    bind_interrupts, dma,
    gpio::{Input, Level, Output, Pull},
    peripherals::{DMA_CH0, DMA_CH1, USB},
    spi::{self, Spi},
    usb::{Driver, InterruptHandler as UsbInterruptHandler},
};
use embassy_time::Delay;
use embassy_usb::{
    class::cdc_acm::{CdcAcmClass, State},
    Builder, Config as UsbConfig, UsbDevice,
};
use embedded_hal_bus::spi::ExclusiveDevice;
use heapless::Vec;
use lora_phy::{
    iv::GenericSx126xInterfaceVariant,
    mod_params::*,
    mod_traits::RadioKind,
    sx126x::{self, Sx1262, Sx126x, TcxoCtrlVoltage},
    LoRa, RxMode,
};
use static_cell::StaticCell;
use {defmt_rtt as _, panic_probe as _};

// ---------------------------------------------------------------------------
// Interrupt bindings
// ---------------------------------------------------------------------------
bind_interrupts!(struct Irqs {
    USBCTRL_IRQ => UsbInterruptHandler<USB>;
    // Both DMA channels used by async Spi::new must be registered here
    // (required since embassy-rp 0.10.0)
    DMA_IRQ_0 => dma::InterruptHandler<DMA_CH0>, dma::InterruptHandler<DMA_CH1>;
});

// ---------------------------------------------------------------------------
// KISS framing constants  (RFC 1055 / TNC-2 KISS)
// ---------------------------------------------------------------------------
const FEND:  u8 = 0xC0; // Frame delimiter (start and end)
const FESC:  u8 = 0xDB; // Escape byte
const TFEND: u8 = 0xDC; // Escaped FEND  (0xDB 0xDC → 0xC0)
const TFESC: u8 = 0xDD; // Escaped FESC  (0xDB 0xDD → 0xDB)

// ---------------------------------------------------------------------------
// KISS / RNode command bytes
// (lower nibble of frame[0]; upper nibble is the port number, always 0 here)
// ---------------------------------------------------------------------------
const CMD_DATA:      u8 = 0x00; // Data frame
const CMD_FREQ:      u8 = 0x01; // Set frequency     (4 bytes big-endian, Hz)
const CMD_BW:        u8 = 0x02; // Set bandwidth     (4 bytes big-endian, Hz)
const CMD_TXPWR:     u8 = 0x03; // Set TX power      (1 byte, dBm)
const CMD_SF:        u8 = 0x04; // Set spreading factor (1 byte, 5-12)
const CMD_CR:        u8 = 0x05; // Set coding rate   (1 byte, 5-8)
// Signal-quality reports sent from firmware → host after each received packet
const CMD_STAT_RSSI: u8 = 0x23; // RSSI report  (1 byte: -rssi_dbm)
const CMD_STAT_SNR:  u8 = 0x24; // SNR  report  (1 byte: (snr_db+32)*4)

// ---------------------------------------------------------------------------
// Static storage for USB descriptor and state buffers.
// All buffers passed to Builder::new must outlive the UsbDevice, which is
// spawned as a 'static task — so they must be 'static themselves.
// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------
// Concrete USB type aliases — defined here so every helper function below
// can reference them without forward-declaration concerns.
// AppUsbDriver = Driver<'static, USB> because all buffers come from StaticCell
// (which hands out &'static mut refs), making the builder lifetime 'static.
// ---------------------------------------------------------------------------
type AppUsbDriver = Driver<'static, USB>;
type AppCdc       = CdcAcmClass<'static, AppUsbDriver>;

static CONFIG_DESCRIPTOR: StaticCell<[u8; 256]> = StaticCell::new();
static BOS_DESCRIPTOR:    StaticCell<[u8; 256]> = StaticCell::new();
static CONTROL_BUF:       StaticCell<[u8; 64]>  = StaticCell::new();
static USB_STATE:         StaticCell<State>      = StaticCell::new();

// ---------------------------------------------------------------------------
// Runtime LoRa configuration — updated via KISS config commands from the host
// ---------------------------------------------------------------------------
struct LoraConfig {
    freq:     u32,           // Hz
    sf:       SpreadingFactor,
    bw:       Bandwidth,
    cr:       CodingRate,
    tx_power: i32,           // dBm
}

impl Default for LoraConfig {
    fn default() -> Self {
        Self {
            freq:     868_200_000,         // 868.2 MHz (EU)
            sf:       SpreadingFactor::_7,
            bw:       Bandwidth::_125KHz,
            cr:       CodingRate::_4_5,
            tx_power: 14,
        }
    }
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------
#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_rp::init(Default::default());

    // =========================================================================
    // USB CDC-ACM
    // =========================================================================
    let usb_driver = Driver::new(p.USB, Irqs);

    let usb_config = {
        let mut c = UsbConfig::new(0x2e8a, 0x000a); // Raspberry Pi VID / RP2040 PID
        c.manufacturer      = Some("Reticulum");
        c.product           = Some("RAK11300 RNode");
        c.serial_number     = Some("00000001");
        c.max_power         = 100; // mA
        c.max_packet_size_0 = 64;
        c
    };

    let mut builder = Builder::new(
        usb_driver,
        usb_config,
        CONFIG_DESCRIPTOR.init([0; 256]), // configuration descriptor buffer
        BOS_DESCRIPTOR.init([0; 256]),    // BOS descriptor buffer
        &mut [],                          // MS OS descriptor — not needed
        CONTROL_BUF.init([0; 64]),        // control endpoint buffer
    );

    let state = USB_STATE.init(State::new());
    let mut cdc = CdcAcmClass::new(&mut builder, state, 64);
    let usb = builder.build();

    // embassy-executor 0.10: task fn returns Result<SpawnToken,_> → unwrap first;
    // spawner.spawn() returns () → no unwrap at end.
    spawner.spawn(unwrap!(usb_task(usb)));

    // =========================================================================
    // SX1262 GPIO + SPI
    // =========================================================================
    let nss   = Output::new(p.PIN_13, Level::High); // chip-select, idle high
    let reset = Output::new(p.PIN_14, Level::High); // reset, idle high
    let busy  = Input::new(p.PIN_15, Pull::None);   // busy flag
    let dio1  = Input::new(p.PIN_29, Pull::None);   // IRQ / DIO1

    // GPIO25 supplies power to the antenna switch.  TX/RX direction is
    // controlled automatically by the SX1262 via DIO2.
    let _rxen = Output::new(p.PIN_25, Level::High);

    // Async (DMA-backed) SPI — Irqs must be provided since embassy-rp 0.10
    let mut spi_cfg = spi::Config::default();
    spi_cfg.frequency = 2_000_000; // 2 MHz (SX1262 max: 16 MHz)
    let spi = Spi::new(
        p.SPI1,
        p.PIN_10,  // SCK
        p.PIN_11,  // MOSI
        p.PIN_12,  // MISO
        p.DMA_CH0, // TX DMA channel
        p.DMA_CH1, // RX DMA channel
        Irqs,      // DMA interrupt binding (required by embassy-rp 0.10)
        spi_cfg,
    );

    // ExclusiveDevice: wraps SpiBus + CS pin into SpiDevice without a Mutex,
    // safe because SX1262 is the only device on SPI1.
    let spi_dev = ExclusiveDevice::new(spi, nss, Delay).unwrap();

    // =========================================================================
    // SX1262 radio init
    // RAK11300 specifics:
    //   • TCXO powered via DIO3 at ~1.7 V
    //   • DIO2 drives the antenna-switch direction automatically
    //   • Module uses DCDC converter (not LDO-only)
    // =========================================================================
    let radio_cfg = sx126x::Config {
        chip:      Sx1262,
        tcxo_ctrl: Some(TcxoCtrlVoltage::Ctrl1V7),
        use_dcdc:  true,
        rx_boost:  false,
    };
    let iv = GenericSx126xInterfaceVariant::new(reset, dio1, busy, None, None)
        .expect("InterfaceVariant");
    let mut lora = LoRa::new(Sx126x::new(spi_dev, iv, radio_cfg), true, Delay)
        .await
        .expect("LoRa init");

    let mut cfg = LoraConfig::default();

    info!("RAK11300 RNode ready");

    // =========================================================================
    // Outer loop — re-enters on USB disconnect/reconnect
    // =========================================================================
    loop {
        cdc.wait_connection().await;
        info!("USB host connected");

        // Build initial modulation + packet parameter sets
        let mut mdltn  = build_mdltn(&mut lora, &cfg);
        let mut tx_pkt = build_tx_pkt(&mut lora, &mdltn);
        let mut rx_pkt = build_rx_pkt(&mut lora, &mdltn);

        // KISS decoder state (persists across USB reads within one connection)
        let mut kiss_frame: Vec<u8, 512> = Vec::new();
        let mut in_frame = false;
        let mut escaped  = false;

        // Receive buffers
        let mut usb_buf = [0u8; 64];
        let mut rx_buf  = [0u8; 255];

        // Start in continuous RX
        radio_enter_rx(&mut lora, &mdltn, &rx_pkt).await;

        // =====================================================================
        // Inner loop — runs while USB host is connected
        // =====================================================================
        'connected: loop {

            // Wait for EITHER a received LoRa packet OR incoming USB bytes.
            // When the USB branch wins the lora.rx() future is dropped — the
            // radio stays in continuous-RX hardware mode and prepare_for_rx()
            // (called at the bottom of every iteration) re-arms it cleanly.
            match select(
                lora.rx(&rx_pkt, &mut rx_buf),
                cdc.read_packet(&mut usb_buf),
            ).await
            {
                // =============================================================
                // Branch A — LoRa packet received
                // =============================================================
                Either::First(rx_result) => {
                    match rx_result {
                        Ok((len, status)) => {
                            info!(
                                "RX {} bytes  RSSI={}dBm  SNR={}dB",
                                len, status.rssi, status.snr
                            );

                            // 1. Forward payload to host as KISS data frame
                            send_data_frame(&mut cdc, &rx_buf[..len as usize]).await;

                            // 2. Signal-quality reports (Reticulum encoding)
                            //    RSSI byte : -rssi_dbm  (e.g. -80 dBm → 80)
                            //    SNR  byte : (snr_db+32)*4  (e.g. 5 dB → 148)
                            let rssi_byte = (-(status.rssi as i32))
                                .clamp(0, 255) as u8;
                            let snr_byte  = ((status.snr as i32 + 32) * 4)
                                .clamp(0, 255) as u8;
                            send_stat_frame(&mut cdc, CMD_STAT_RSSI, rssi_byte).await;
                            send_stat_frame(&mut cdc, CMD_STAT_SNR,  snr_byte).await;
                        }
                        Err(_) => warn!("LoRa RX error"),
                    }
                }

                // =============================================================
                // Branch B — bytes arrived from USB host
                // =============================================================
                Either::Second(usb_result) => {
                    let n = match usb_result {
                        Ok(n)  => n,
                        Err(_) => {
                            info!("USB disconnected");
                            break 'connected;
                        }
                    };

                    // ── KISS byte-by-byte state machine ──────────────────────
                    let mut tx_payload: Option<Vec<u8, 255>> = None;
                    let mut config_changed = false;

                    for &byte in &usb_buf[..n] {

                        // FESC escape decoding
                        if escaped {
                            escaped = false;
                            let decoded = match byte {
                                TFEND => FEND,
                                TFESC => FESC,
                                _     => byte, // malformed — pass through
                            };
                            if in_frame { let _ = kiss_frame.push(decoded); }
                            continue;
                        }

                        match byte {
                            // Frame boundary
                            FEND => {
                                escaped = false;
                                if in_frame && !kiss_frame.is_empty() {
                                    // Lower nibble = command, upper nibble = port (ignored)
                                    let cmd = kiss_frame[0] & 0x0F;
                                    match cmd {
                                        CMD_DATA => {
                                            // Copy payload out before clearing the frame
                                            let mut v: Vec<u8, 255> = Vec::new();
                                            for &b in kiss_frame
                                                .iter()
                                                .skip(1)
                                                .take(255)
                                            {
                                                let _ = v.push(b);
                                            }
                                            tx_payload = Some(v);
                                        }
                                        CMD_FREQ if kiss_frame.len() >= 5 => {
                                            cfg.freq = u32::from_be_bytes([
                                                kiss_frame[1], kiss_frame[2],
                                                kiss_frame[3], kiss_frame[4],
                                            ]);
                                            info!("Set freq → {} Hz", cfg.freq);
                                            config_changed = true;
                                        }
                                        CMD_BW if kiss_frame.len() >= 5 => {
                                            let hz = u32::from_be_bytes([
                                                kiss_frame[1], kiss_frame[2],
                                                kiss_frame[3], kiss_frame[4],
                                            ]);
                                            cfg.bw = bw_from_hz(hz);
                                            info!("Set BW → {} Hz", hz);
                                            config_changed = true;
                                        }
                                        CMD_SF if kiss_frame.len() >= 2 => {
                                            cfg.sf = sf_from_byte(kiss_frame[1]);
                                            info!("Set SF → {}", kiss_frame[1]);
                                            config_changed = true;
                                        }
                                        CMD_CR if kiss_frame.len() >= 2 => {
                                            cfg.cr = cr_from_byte(kiss_frame[1]);
                                            info!("Set CR → {}", kiss_frame[1]);
                                            config_changed = true;
                                        }
                                        CMD_TXPWR if kiss_frame.len() >= 2 => {
                                            cfg.tx_power = kiss_frame[1] as i32;
                                            // TX power is read at TX time; no rebuild needed
                                            info!("Set TX pwr → {} dBm", cfg.tx_power);
                                        }
                                        _ => {}
                                    }
                                }
                                kiss_frame.clear();
                                in_frame = true;
                            }

                            FESC => {
                                if in_frame { escaped = true; }
                            }

                            _ => {
                                if in_frame {
                                    // Silently drop byte if frame buffer is full
                                    // (shouldn't happen: 512 B >> 255 B LoRa max)
                                    let _ = kiss_frame.push(byte);
                                }
                            }
                        }
                    } // end byte loop

                    // ── Apply config changes ──────────────────────────────────
                    if config_changed {
                        mdltn  = build_mdltn(&mut lora, &cfg);
                        tx_pkt = build_tx_pkt(&mut lora, &mdltn);
                        rx_pkt = build_rx_pkt(&mut lora, &mdltn);
                    }

                    // ── Transmit if we decoded a data frame ───────────────────
                    if let Some(ref payload) = tx_payload {
                        if !payload.is_empty() {
                            match lora
                                .prepare_for_tx(&mdltn, &mut tx_pkt, cfg.tx_power, payload)
                                .await
                            {
                                Ok(()) => match lora.tx().await {
                                    Ok(())  => info!("TX OK — {} bytes", payload.len()),
                                    Err(_)  => error!("TX failed"),
                                },
                                Err(_) => error!("prepare_for_tx failed"),
                            }
                        }
                    }
                } // end Either::Second
            } // end select

            // Re-arm continuous RX at the bottom of every iteration:
            //   • After LoRa RX: radio left standby after packet interrupt
            //   • After TX:      radio left standby after transmission
            //   • After USB-only (no TX): radio still in continuous RX hardware
            //     mode, but calling prepare_for_rx() here is idempotent and
            //     keeps the lora-phy state machine consistent.
            radio_enter_rx(&mut lora, &mdltn, &rx_pkt).await;

        } // end 'connected
    } // end outer loop
}

// ---------------------------------------------------------------------------
// Radio helpers
// ---------------------------------------------------------------------------

fn build_mdltn<RK: RadioKind>(
    lora: &mut LoRa<RK, Delay>,
    cfg:  &LoraConfig,
) -> ModulationParams {
    lora.create_modulation_params(cfg.sf, cfg.bw, cfg.cr, cfg.freq)
        .expect("build_mdltn")
}

fn build_tx_pkt<RK: RadioKind>(
    lora:  &mut LoRa<RK, Delay>,
    mdltn: &ModulationParams,
) -> PacketParams {
    lora.create_tx_packet_params(
        8,     // preamble symbols
        false, // explicit header
        true,  // CRC on
        false, // IQ not inverted
        mdltn,
    )
    .expect("build_tx_pkt")
}

fn build_rx_pkt<RK: RadioKind>(
    lora:  &mut LoRa<RK, Delay>,
    mdltn: &ModulationParams,
) -> PacketParams {
    lora.create_rx_packet_params(
        8,     // preamble symbols
        false, // explicit header
        255,   // max payload length (SX1262 hardware maximum)
        true,  // CRC on
        false, // IQ not inverted
        mdltn,
    )
    .expect("build_rx_pkt")
}

async fn radio_enter_rx<RK: RadioKind>(
    lora:   &mut LoRa<RK, Delay>,
    mdltn:  &ModulationParams,
    rx_pkt: &PacketParams,
) {
    if let Err(_) = lora
        .prepare_for_rx(RxMode::Continuous, mdltn, rx_pkt)
        .await
    {
        error!("prepare_for_rx failed");
    }
}

// ---------------------------------------------------------------------------
// KISS output helpers (firmware → host)
// ---------------------------------------------------------------------------

/// Send a received LoRa payload as a KISS data frame to the USB host.
async fn send_data_frame(
    cdc:     &mut AppCdc,
    payload: &[u8],
) {
    // Worst case: every byte is FEND or FESC → 2 bytes each.
    // 255 × 2  +  1 (FEND)  +  1 (CMD_DATA)  +  1 (FEND) = 513 bytes.
    let mut frame: Vec<u8, 516> = Vec::new();
    let _ = frame.push(FEND);
    let _ = frame.push(CMD_DATA);
    for &b in payload {
        match b {
            FEND => { let _ = frame.push(FESC); let _ = frame.push(TFEND); }
            FESC => { let _ = frame.push(FESC); let _ = frame.push(TFESC); }
            _    => { let _ = frame.push(b); }
        }
    }
    let _ = frame.push(FEND);
    cdc_write(cdc, &frame).await;
}

/// Send a one-byte signal-quality stat frame (RSSI or SNR) to the USB host.
async fn send_stat_frame(
    cdc:   &mut AppCdc,
    cmd:   u8,
    value: u8,
) {
    // Frame: FEND  cmd  [escaped value]  FEND
    let (buf, len): ([u8; 5], usize) = match value {
        FEND => ([FEND, cmd, FESC, TFEND, FEND], 5),
        FESC => ([FEND, cmd, FESC, TFESC, FEND], 5),
        _    => ([FEND, cmd, value,  FEND,    0], 4),
    };
    let _ = cdc.write_packet(&buf[..len]).await;
}

/// Write arbitrary bytes to CDC, splitting into ≤64-byte USB packets.
/// Sends a zero-length packet when the total length is a multiple of 64,
/// so the host receives a definite end-of-transfer.
async fn cdc_write(cdc: &mut AppCdc, data: &[u8]) {
    for chunk in data.chunks(64) {
        if cdc.write_packet(chunk).await.is_err() {
            return;
        }
    }
    if data.len() % 64 == 0 {
        let _ = cdc.write_packet(&[]).await;
    }
}

// ---------------------------------------------------------------------------
// Enum converters  (KISS byte values → lora-phy enum variants)
// ---------------------------------------------------------------------------

/// Map a bandwidth value in Hz to the nearest supported lora-phy Bandwidth.
fn bw_from_hz(hz: u32) -> Bandwidth {
    match hz {
        0        ..= 9_000   => Bandwidth::_7KHz,
        9_001    ..= 12_000  => Bandwidth::_10KHz,
        12_001   ..= 18_000  => Bandwidth::_15KHz,
        18_001   ..= 26_000  => Bandwidth::_20KHz,
        26_001   ..= 36_000  => Bandwidth::_31KHz,
        36_001   ..= 52_000  => Bandwidth::_41KHz,
        52_001   ..= 93_000  => Bandwidth::_62KHz,
        93_001   ..= 187_000 => Bandwidth::_125KHz,
        187_001  ..= 375_000 => Bandwidth::_250KHz,
        _                    => Bandwidth::_500KHz,
    }
}

/// Map a spreading-factor byte (5-12) to a lora-phy SpreadingFactor.
fn sf_from_byte(sf: u8) -> SpreadingFactor {
    match sf {
        5  => SpreadingFactor::_5,
        6  => SpreadingFactor::_6,
        7  => SpreadingFactor::_7,
        8  => SpreadingFactor::_8,
        9  => SpreadingFactor::_9,
        10 => SpreadingFactor::_10,
        11 => SpreadingFactor::_11,
        12 => SpreadingFactor::_12,
        _  => SpreadingFactor::_7, // default
    }
}

/// Map a coding-rate byte (5-8, meaning CR 4/5 to 4/8) to a lora-phy CodingRate.
fn cr_from_byte(cr: u8) -> CodingRate {
    match cr {
        5 => CodingRate::_4_5,
        6 => CodingRate::_4_6,
        7 => CodingRate::_4_7,
        8 => CodingRate::_4_8,
        _ => CodingRate::_4_5, // default
    }
}

// ---------------------------------------------------------------------------
// USB background task — must never return
// ---------------------------------------------------------------------------
#[embassy_executor::task]
async fn usb_task(mut usb: UsbDevice<'static, AppUsbDriver>) -> ! {
    usb.run().await
}
