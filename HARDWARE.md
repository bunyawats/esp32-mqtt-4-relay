# Hardware: DIYmall ESP32S 4-Channel WiFi Bluetooth Relay Module

Summary of the vendor manual ("DIYmall ESP32S 4 Channel WiFi Bluetooth Relay Module User Manual",
manuals.plus, saved 2026-09-17), plus notes on what it means for this firmware. The original PDF
is not checked into this repo.

## Board

| Item | Value |
|---|---|
| Model | FZ2707U |
| PCB marking | Relay32 V2.1 (underside) |
| MCU module | ESP32-WROOM-32U: classic dual-core Xtensa ESP32, build target `xtensa-esp32-espidf` |
| Antenna | External 2.4 GHz antenna on the module's U.FL connector (the -32U has no PCB antenna, so the antenna must be attached for WiFi to work) |
| Relays | 4 independently controlled SONGLE SRD-05VDC-SL-C |
| Relay contact rating | 250 VAC / 10 A, 125 VAC / 10 A, 30 VDC / 10 A |
| Power input | 6 V 0.75 A per spec; manual recommends 9 V 1 A or 12 V 1 A if the board misbehaves. Supplied via DC barrel jack or a 2-pin screw terminal |
| On-board buttons | RESET, DOWNLOAD |
| USB | None. Programming goes through an external USB-to-TTL adapter |
| Wireless | WiFi 2.4 GHz, Bluetooth |

Kit contents: the relay module, a USB-to-TTL serial adapter, jumper wires, the 2.4G antenna, and
optionally a US or EU plug power adapter.

## Relay GPIO mapping: **not documented**

The manual does not say which ESP32 GPIO drives each relay, whether the relays turn on with a
high or low signal, or which GPIO (if any) drives a status LED. Neither the text nor the photos
show it. The values in `src/main.rs` (relays 1–4 on GPIO 32, 33, 25, 26, `RELAY_ACTIVE_HIGH = true`)
are **placeholders** copied from a different common board layout.

Update this table once the pins are confirmed on the real board, whether by probing, by tracing
the PCB, or from a trusted pinout:

| Relay | GPIO | Active level | Confirmed? |
|---|---|---|---|
| 1 | ? | ? | no |
| 2 | ? | ? | no |
| 3 | ? | ? | no |
| 4 | ? | ? | no |

### Finding the pins with `pin_probe`

`src/bin/pin_probe.rs` is a separate test firmware. It needs no WiFi, MQTT or `cfg.toml`. It takes
each safe-to-drive GPIO in turn and drives it HIGH for 3 s, then LOW for 3 s, then releases it,
logging each step over serial. One pass takes about 2.5 minutes, and it repeats forever.

1. **Disconnect everything from the relay contacts.** The probe will switch relays unpredictably.
2. Back up the stock firmware first (see below).
3. Build and flash. Put the board in download mode first by holding DOWNLOAD and tapping RESET:
   ```bash
   cargo build --release --bin pin_probe
   espflash flash -p <port> target/xtensa-esp32-espidf/release/pin_probe
   ```
4. Tap RESET, run `espflash monitor -p <port>`, and watch the relays and their LEDs:
   - A relay that turns on during a `GPIO n -> HIGH` line is active-high on GPIO n.
   - A relay that turns on during a `GPIO n -> LOW` line is active-low on GPIO n.

   If every relay is on at boot and turns *off* at some step, that also means active-low.
5. Fill in the table above. Then update the pin list and `RELAY_ACTIVE_HIGH` in `src/main.rs`, and
   flash the real firmware.

## Programming / flashing

Wiring from the board's header to the USB-TTL adapter, per the manual:

| Relay32 | USB-TTL adapter |
|---|---|
| GND | GND |
| TXD | RXD |
| RXD | TXD |

Power the board from its own supply (DC jack or screw terminal) while programming. The manual only
shows GND/TX/RX going to the adapter.

The board has no auto-reset/auto-boot circuit, so boot mode is entered by hand:

1. Hold **DOWNLOAD** (pulls GPIO0 low), tap **RESET**, then release DOWNLOAD.
2. Flash: `espflash flash -p <port> target/xtensa-esp32-espidf/release/esp32-mqtt-relay`
3. Tap **RESET** to run the new firmware, then `espflash monitor -p <port>`. espflash can't reset
   the chip itself on this board, so expect to press RESET to see the boot log.

The serial port is the USB-TTL adapter's, e.g. `/dev/cu.usbserial-*` or `/dev/cu.wchusbserial-*`.

## Stock (factory) firmware

Know this so you can recognize the stock firmware, or go back to it:

- Runs only as a WiFi **access point**: SSID `TEST`, password `123456789` (WPA2). It cannot join
  an existing network, which is why this project replaces it.
- HTTP control at `192.168.1.1`:
  - `/h1` … `/h4`: relay N on
  - `/l1` … `/l4`: relay N off
  - `/j`: all relay states as JSON
- A Bluetooth GATT server runs alongside the AP. The iOS app is "DIYMall Relay Control"; the
  Android app is available from the seller.
- The seller can provide the firmware on request. It is not otherwise published, so **back it up
  before flashing ours**. With the board in download mode:

  ```bash
  espflash read-flash -p <port> 0 0x400000 stock-firmware.bin   # WROOM-32U is typically 4 MB; espflash board-info confirms
  ```

  To restore it: `espflash write-bin -p <port> 0 stock-firmware.bin`.

## Troubleshooting (from the manual)

- **Won't power on / erratic:** the supply is too weak. Try 9 V 1 A or 12 V 1 A.
- **Unresponsive after a custom flash:** recheck the GND/TXD/RXD wiring (TX and RX crossed), make
  sure the board was put into download mode, and make sure the build actually succeeded.
- **Poor WiFi:** make sure the antenna is firmly seated on the U.FL connector and not blocked by
  anything.
