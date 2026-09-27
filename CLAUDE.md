# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

ESP32 firmware for a 4-channel relay board, controlled over MQTT (`<prefix>/1..4` ← `on|off|toggle`,
JSON status on a separate topic). It was derived from the earlier `esp32-mqtt-blink` project;
the MQTT threading design carries over, and the blink/LED logic was removed. The README has the
user-facing MQTT contract.

Target hardware is the DIYmall ESP32S 4-channel relay module (FZ2707U, "Relay32 V2.1", ESP32-WROOM-32U).
`HARDWARE.md` covers specs, flashing (no USB and no auto-reset: flashing needs a USB-TTL adapter and
the DOWNLOAD/RESET buttons), and the stock firmware. The vendor manual does **not** document the
relay GPIOs, so the pins in `src/main.rs` and `RELAY_ACTIVE_HIGH` are **placeholders**. Once they're
confirmed, update the table in `HARDWARE.md`. `src/bin/pin_probe.rs` is a standalone test
firmware for finding them (`cargo build --release --bin pin_probe`). It shares nothing with `main.rs`.

For toolchain, flashing and MQTT troubleshooting, use the global `esp32-rust-idf` skill (firmware
side) and `local-mqtt-broker` skill (broker/network side).

## Build & flash

Stack: Rust `std` on ESP-IDF (`esp-idf-svc` 0.52, ESP-IDF v5.2.2). `rust-toolchain.toml` pins the
`esp` toolchain. `.cargo/config.toml` pins target `xtensa-esp32-espidf`, the `ldproxy` linker and
`build-std`. For a different chip, change `target` and `MCU` there together.

```bash
. $HOME/export-esp.sh          # espup env; required in every new shell
cargo build --release
cargo clippy --release
espflash flash --monitor target/xtensa-esp32-espidf/release/esp32-mqtt-relay
```

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
