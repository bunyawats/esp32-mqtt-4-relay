# AGENTS.md

This file provides guidance to Codex (Codex.ai/code) when working with code in this repository.

## Project

ESP32 firmware for a 2-channel relay board, controlled over MQTT (`<prefix>/1..2` ← `on|off|toggle`,
JSON status on a separate topic). It was derived from the earlier `esp32-mqtt-blink` project;
the MQTT threading design carries over, and the blink/LED logic was removed. The README has the
user-facing MQTT contract.

Target hardware is the "ESP32 Relay X2" board, AC 220 V variant (ESP32-WROOM-32E, relays on
GPIO16/17 active-high, LED on GPIO23; reference: https://devices.esphome.io/devices/esp32-relay-x2/).
The repo name is left over from an earlier 4-relay target. `HARDWARE.md` covers the GPIO map,
flashing (no USB and no auto-reset: flashing needs a USB-TTL adapter and the IO0/EN buttons), and
the stock firmware backup. `src/bin/pin_probe.rs` is a standalone test firmware for finding relay
pins (`cargo build --release --bin pin_probe`). It shares nothing with `main.rs`.

The WCH USB-TTL adapter in use can't flash with espflash (it corrupts long host-to-board bursts).
Flash with esptool and gapped writes instead, as described in `HARDWARE.md`.

For toolchain, flashing and MQTT troubleshooting, use the global `esp32-rust-idf` skill (firmware
side) and `local-mqtt-broker` skill (broker/network side).

## Build & flash

Stack: Rust `std` on ESP-IDF (`esp-idf-svc` 0.52, ESP-IDF v5.2.2). `rust-toolchain.toml` pins the
`esp-1.98` toolchain (1.98.1.0), because the 1.99.0.0 esp release fails to build `std` for espidf
(`AT_FDCWD` not found in `libc`). Install it with
`espup install --toolchain-version 1.98.1.0 --name esp-1.98 --targets esp32`. `.cargo/config.toml`
pins target `xtensa-esp32-espidf`, the `ldproxy` linker and `build-std`. For a different chip, change `target` and `MCU` there together.

```bash
. $HOME/export-esp.sh          # espup env; required in every new shell
cargo build --release
cargo clippy --release
```

To flash, put the board in download mode (hold IO0, tap EN), write a merged image with the
gapped esptool from `HARDWARE.md`, then power-cycle the board (no buttons) to run it.

The repo has no tests, and host `cargo test` doesn't work with this target config. To check
behavior, flash the board and drive it with `mosquitto_pub`/`mosquitto_sub`.

## Configuration

`cfg.toml` (gitignored; copy it from `cfg.toml.example`) is read **at compile time** by `toml-cfg`.
The section name `[esp32-mqtt-relay]` must match the crate name. `topic_status` must not be under
`topic_relay_prefix/`, because the device subscribes to `<prefix>/+`.

## Architecture (`src/main.rs`)

- **Main thread** is the only owner of the relay `PinDriver`s. It sets every relay off before
  WiFi starts, then blocks on a command channel and applies each `Command`. It mirrors state into
  the `relay_bits` `AtomicU8` bitmask (bit 0 = relay 1), which the other threads read for status.
- **Connection-draining thread** pumps `connection.next()` continuously (esp-rs/esp-idf-svc#441).
  It parses topics and payloads and forwards them over `mpsc` channels. It must **never call
  `client.publish()`/`subscribe()` itself**, because that deadlocks. On `Disconnected` it clears
  `subscribed`.
- **Subscriber thread** polls `subscribed` and retries `subscribe("<prefix>/+")` every 500ms while
  it is false. That covers both boot and resubscribing after a reconnect (clean session). Don't
  gate subscribe on a `Connected` event; that has been seen to hang.
- **WiFi watchdog thread** owns the `BlockingWifi` after boot. esp-idf-svc doesn't reconnect the
  station by itself, so every 5s it checks `is_up()` and reconnects if needed. It never touches the relays.
  A failed connect at boot is logged and left to the watchdog instead of aborting `main`.
- **Status-publisher thread** is the only caller of `publish()`. Every status goes through
  `status_tx`, including the 30s heartbeat.
