   # esp32-mqtt-relay

ESP32 firmware (Rust, `esp-idf-svc` std stack) for a 2-channel ESP32 relay board ("ESP32 Relay
X2"), controlled over MQTT. See [HARDWARE.md](HARDWARE.md) for the board, its GPIO map, the
flashing procedure, and the stock firmware backup. The
[ESPHome device page](https://devices.esphome.io/devices/esp32-relay-x2/) is a good reference for
the board itself.

Each relay has its own topic and accepts `on`, `off` or `toggle`. Any MQTT client works
(`mosquitto_pub`, Home Assistant, an AI agent shelling out, ...).

All relays are driven **off** at boot, before WiFi even starts, and stay off until a command
arrives. Losing the broker or WiFi never changes relay state: the device retries WiFi every 5s
and reconnects/resubscribes to MQTT on its own. Commands sent while it's offline are lost.

## Prerequisites

```bash
cargo install espup
espup install
espup install --toolchain-version 1.98.1.0 --name esp-1.98 --targets esp32
. $HOME/export-esp.sh   # every new shell

cargo install espflash ldproxy
```

`rust-toolchain.toml` pins the `esp-1.98` toolchain (the 1.99.0.0 esp release can't build `std`
for ESP-IDF), and `.cargo/config.toml` pins the `xtensa-esp32-espidf` target, so `cargo build` targets the ESP32 out of the box. For a
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

2. **Relay pins.** `src/main.rs` drives relays 1–2 on GPIO16 and GPIO17, active-high. For a
   different board, change `RELAY_COUNT`, the pin list and `RELAY_ACTIVE_HIGH`; `pin_probe`
   (see HARDWARE.md) finds the pins.

## Build & flash

```bash
cargo build --release
# board has no USB/auto-reset: wire a USB-TTL adapter, hold IO0 + tap EN first
espflash flash -p <port> --before no-reset --after no-reset \
  target/xtensa-esp32-espidf/release/esp32-mqtt-relay
# then power-cycle the board (no buttons), and:
espflash monitor -p <port> --before no-reset
```

Some USB-TTL adapters can't flash with espflash at all. HARDWARE.md has the esptool workaround.

## Usage

```bash
mosquitto_pub -h your-broker-host.local -t esp32/relay/1 -m on
mosquitto_pub -h your-broker-host.local -t esp32/relay/2 -m off
mosquitto_pub -h your-broker-host.local -t esp32/relay/1 -m toggle
```

Watch status. It's published after every command or rejection, on each (re)subscribe to the
broker, and every 30s as a heartbeat:

```bash
mosquitto_sub -h your-broker-host.local -t esp32/relay_status
```

```json
{"relays":["on","off"],"relay":1,"reason":"switch_on"}
{"relays":["on","off"],"relay":null,"reason":"heartbeat"}
{"relays":["on","off"],"relay":2,"reason":"rejected_invalid_command"}
```

`relays[i]` is the state of relay `i+1`. `relay` is the relay the event was about (`null` for
`heartbeat`, `connected`, and `rejected_invalid_relay`). Reasons are `switch_on`, `switch_off`,
`switch_toggle`, `connected`, `heartbeat`, `rejected_invalid_relay` (a topic like
`esp32/relay/3`), and `rejected_invalid_command` (a payload other than `on|off|toggle`).

### Testing on the board

On the broker host itself, `-h localhost` works. Watch status in one terminal:

```bash
mosquitto_sub -h your-broker-host.local -t esp32/relay_status -v
```

Then, in another terminal, switch each relay and listen for the click:

```bash
mosquitto_pub -h your-broker-host.local -t esp32/relay/1 -m on       # relay 1 (outermost) on
mosquitto_pub -h your-broker-host.local -t esp32/relay/1 -m off
mosquitto_pub -h your-broker-host.local -t esp32/relay/2 -m on       # relay 2 on
mosquitto_pub -h your-broker-host.local -t esp32/relay/2 -m off
mosquitto_pub -h your-broker-host.local -t esp32/relay/1 -m toggle
```

Check that bad input is rejected:

```bash
mosquitto_pub -h your-broker-host.local -t esp32/relay/3 -m on       # rejected_invalid_relay
mosquitto_pub -h your-broker-host.local -t esp32/relay/1 -m blah     # rejected_invalid_command
```

Or cycle both relays, 5 s on each (works in bash and zsh):

```bash
for relay in 1 2; do
  echo ">> $(date +%H:%M:%S) relay $relay on"
  mosquitto_pub -h your-broker-host.local -t esp32/relay/$relay -m on;  sleep 5
  echo ">> $(date +%H:%M:%S) relay $relay off"
  mosquitto_pub -h your-broker-host.local -t esp32/relay/$relay -m off; sleep 4
done
```

## Notes / next steps

- WiFi and MQTT credentials are embedded in the firmware binary. For stronger isolation, move to
  NVS-based provisioning.
- Relay state is not persisted: after a power cycle all relays are off.
- Consider a retained status message and an MQTT Last Will so subscribers see the last state
  immediately and learn when the device drops offline.
