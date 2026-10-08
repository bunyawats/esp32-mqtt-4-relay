# Hardware: ESP32 Relay X2 (2-channel, AC 220 V)

The firmware targets a 2-relay ESP32 board with no USB: the board ESPHome lists as
[ESP32 Relay X2](https://devices.esphome.io/devices/esp32-relay-x2/), in its AC 220 V variant.
The [fernandorpardo/ESP32-X2-relay](https://github.com/fernandorpardo/ESP32-X2-relay) project
targets the same board.

![ESP32 2-channel relay board with AC 220 V input](ESP32-X2-relay%20AC.png)

## Board

| Item | Value |
|---|---|
| Maker marking | "LC" (LC Technology), PCB dated 2024-09-06 |
| MCU module | ESP32-WROOM-32E (PCB antenna); chip ESP32-D0WD-V3, revision v3.1: classic dual-core Xtensa ESP32, build target `xtensa-esp32-espidf` |
| Flash | 4 MB |
| Relays | 2 × SONGLE SRD-05VDC-SL-C (10 A 250 VAC / 10 A 30 VDC), COM/NO/NC screw terminals |
| On-board buttons | **IO0** (download mode, GPIO0) and **EN** (reset) |
| USB | None. Programming goes through an external USB-to-TTL adapter on the 6-pin serial header |
| Power | **AC 220 V** through an on-board AC-DC supply (2-pin screw terminal). 5 V on the header's 5V pin also works, and is what's used for flashing |

ESPHome's page describes a DC variant (7–30 VDC or 5 VDC on separate terminals). This board is
the AC variant, with the same ESP32 side and pinout.

⚠️ The AC input and the relay terminals carry mains voltage. Never connect 220 V while the USB-TTL
adapter is attached or the board is powered from the 5V pin, and keep mains wiring off the relay
contacts while working on the board.

## GPIO mapping

| Function | GPIO | Active level | Used by firmware |
|---|---|---|---|
| Relay 1 (outermost) | 16 | high | `esp32/relay/1` |
| Relay 2 | 17 | high | `esp32/relay/2` |
| LED | 23 | high | not yet |
| Button | 0 | low when pressed | no (it's the IO0 download-mode button) |

Sources that agree: the ESPHome device page (no `inverted:` on the relays, so active-high),
fernandorpardo/ESP32-X2-relay's `main/config.h`, and `pin_probe` on this board. On 2026-10-08 the
main firmware switched both relays on and off over MQTT. Which physical relay is "outermost" is
from ESPHome and wasn't checked separately.

### Re-probing with `pin_probe`

`src/bin/pin_probe.rs` is a separate test firmware. It needs no WiFi, MQTT or `cfg.toml`. It takes
each safe-to-drive GPIO in turn and drives it HIGH for 3 s, then LOW for 3 s, then releases it,
logging each step over serial. One pass takes about 2.5 minutes, and it repeats forever.
Disconnect everything from the relay contacts first: the probe switches relays unpredictably.

```bash
cargo build --release --bin pin_probe
```

Flash it the same way as the main firmware (see below), reset the board, and match relay clicks
to the log's `GPIO n -> HIGH`/`LOW` lines. A relay that turns on during HIGH is active-high.

## Programming / flashing

The 6-pin header carries GND, 5V, TX, RX, GND and IO0 (per fernandorpardo/ESP32-X2-relay). ESPHome
notes it is **not** an FTDI-compatible pinout, so wire it by the silkscreen labels rather than
plugging an adapter straight on:

| Board | USB-TTL adapter |
|---|---|
| GND | GND |
| TX | RX |
| RX | TX |
| 5V | 5V, or leave unconnected if the board has another 5 V supply |

Set the adapter to 3.3 V logic. Use short, firm jumper wires. Loose Dupont contacts made the link
intermittent here, and the adapter dropped off USB several times while the board was being
handled.

The board has no auto-reset/auto-boot circuit, so boot mode is entered by hand:

1. Hold **IO0**, tap **EN**, then release IO0. The serial port shows
   `boot:0x3 (DOWNLOAD_BOOT...) waiting for download`.
2. Flash with `--before no-reset --after no-reset` (see below).
3. Run the new firmware by briefly removing 5 V power without touching any button. Tapping
   **EN** alone also works, but here it often caught IO0 and went straight back into
   `DOWNLOAD_BOOT`.

With a well-behaved adapter (FTDI, CP2102):

```bash
espflash flash -p <port> --before no-reset --after no-reset \
  target/xtensa-esp32-espidf/release/esp32-mqtt-relay
espflash monitor -p <port> --before no-reset
```

### The WCH adapter needs gapped writes

The WCH adapter used here shows up as `/dev/cu.usbmodem*` under macOS's generic driver. It
corrupts any continuous host-to-board burst longer than about 350 bytes, at any baud rate.
Short commands work, so `espflash board-info --no-stub` connects, but every stub upload fails
with `MemData` (espflash) or `0107: Checksum error` (esptool). Sending in 256-byte chunks with
short idle gaps between them works reliably. espflash has no option for that, so flash with
esptool and a patched `serial.Serial.write`. The global `esp32-rust-idf` skill
(`references/usb-ttl-flashing.md`) has the script:

```bash
espflash save-image --chip esp32 --merge --flash-size 4mb \
  target/xtensa-esp32-espidf/release/esp32-mqtt-relay merged.bin
E=.embuild/espressif/python_env/idf5.2_py3.14_env/bin
CHUNK=256 GAP=0.005 $E/python esptool_gapped.py --chip esp32 -p <port> -b 115200 \
  --before no_reset --after no_reset write_flash 0x0 merged.bin
```

A 4 MB merged image takes about 2 minutes this way. A different adapter, or WCH's own macOS
driver (ports named `cu.wchusbserial*`), may not need this.

## Stock (factory) firmware

The board shipped with Espressif's **AT command firmware** (`Bin version(Wroom32):1.1.2`, built
on ESP-IDF v4.4.4), not a relay app. A full 4 MB backup is in `stock-firmware.bin` in the repo
root (SHA-256 `e092c1f17c28c50c6f2c0ec4f1130b80a9e349c1f122fd0831ec32b7b596ad8f`). Keep it out of
git. To restore it, write it at offset 0 the same way as a merged image above.

The ESP32 ROM loader can't read flash, so a backup needs the flasher stub. With the WCH adapter,
that means the gapped esptool with `read_flash 0 0x400000 stock-firmware.bin` (about 6.5 minutes
at 115200 baud).

## References

- [ESPHome device page: ESP32 Relay X2](https://devices.esphome.io/devices/esp32-relay-x2/):
  pinout, relay ratings, power options, header notes, and a working ESPHome config. The best
  single reference for this board.
- [fernandorpardo/ESP32-X2-relay](https://github.com/fernandorpardo/ESP32-X2-relay): ESP-IDF C
  firmware for the same board (REST + MQTT). Its pins are in `main/config.h`, and its README
  describes the 6-pin header.
