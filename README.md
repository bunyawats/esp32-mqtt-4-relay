# esp32-mqtt-relay

ESP32 firmware (Rust, `esp-idf-svc` std stack) for the DIYmall ESP32S 4-channel relay module
(FZ2707U), controlled over MQTT. See [HARDWARE.md](HARDWARE.md) for board specs, the flashing
procedure, and how to back up the stock firmware.

Each relay has its own topic and accepts `on`, `off` or `toggle`. Any MQTT client works
(`mosquitto_pub`, Home Assistant, an AI agent shelling out, ...).

All relays are driven **off** at boot, before WiFi even starts, and stay off until a command
arrives. Losing the broker or WiFi never changes relay state: the device retries WiFi every 5s
and reconnects/resubscribes to MQTT on its own. Commands sent while it's offline are lost.

## Prerequisites

```bash
cargo install espup
espup install
. $HOME/export-esp.sh   # every new shell

cargo install espflash ldproxy
```

`rust-toolchain.toml` and `.cargo/config.toml` pin the `esp` toolchain and the
`xtensa-esp32-espidf` target, so `cargo build` targets the ESP32 out of the box. For a
non-ESP32 chip (C3/S3/...), change `target` and `MCU` there.

## Setup

1. Copy the config template and fill it in:

   ```bash
   cp cfg.toml.example cfg.toml
   ```

   ```toml
   [esp32-mqtt-relay]
   wifi_ssid = "YourWiFiName"
   wifi_pass = "YourWiFiPassword"
   mqtt_url = "mqtt://your-broker-host.local:1883"
   mqtt_client_id = "esp32-relay"
   topic_relay_prefix = "esp32/relay"
   topic_status = "esp32/relay_status"
   ```

   `cfg.toml` is gitignored and is compiled into the firmware, so any change needs a rebuild
   and reflash. Prefer a `.local` mDNS hostname for `mqtt_url` over a literal IP so a DHCP
   change on the broker host doesn't break the device. `topic_status` must not sit under
   `topic_relay_prefix/`, or the device would receive its own status messages.

2. **Check the relay pins.** `src/main.rs` currently uses GPIO32, 33, 25, 26 for relays 1–4
   (the common "ESP32 Relay X4" layout) as placeholders. Confirm them against your board, and set
   `RELAY_ACTIVE_HIGH = false` if your relays energize on a low GPIO (typical on
   opto-isolated boards).

## Build & flash

```bash
cargo build --release
# board has no USB/auto-reset: wire a USB-TTL adapter, hold DOWNLOAD + tap RESET first
espflash flash -p <port> target/xtensa-esp32-espidf/release/esp32-mqtt-relay
# then tap RESET, and:
espflash monitor -p <port>
```

## Usage

```bash
mosquitto_pub -h your-broker-host.local -t esp32/relay/1 -m on
mosquitto_pub -h your-broker-host.local -t esp32/relay/3 -m off
mosquitto_pub -h your-broker-host.local -t esp32/relay/4 -m toggle
```

Watch status. It's published after every command or rejection, on each (re)subscribe to the
broker, and every 30s as a heartbeat:

```bash
mosquitto_sub -h your-broker-host.local -t esp32/relay_status
```

```json
{"relays":["on","off","off","off"],"relay":1,"reason":"switch_on"}
{"relays":["on","off","off","off"],"relay":null,"reason":"heartbeat"}
{"relays":["on","off","off","off"],"relay":2,"reason":"rejected_invalid_command"}
```

`relays[i]` is the state of relay `i+1`. `relay` is the relay the event was about (`null` for
`heartbeat`, `connected`, and `rejected_invalid_relay`). Reasons are `switch_on`, `switch_off`,
`switch_toggle`, `connected`, `heartbeat`, `rejected_invalid_relay` (a topic like
`esp32/relay/7`), and `rejected_invalid_command` (a payload other than `on|off|toggle`).

## Notes / next steps

- WiFi and MQTT credentials are embedded in the firmware binary. For stronger isolation, move to
  NVS-based provisioning.
- Relay state is not persisted: after a power cycle all relays are off.
- Consider a retained status message and an MQTT Last Will so subscribers see the last state
  immediately and learn when the device drops offline.
