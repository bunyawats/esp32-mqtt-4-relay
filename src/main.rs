use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::hal::gpio::{Output, PinDriver};
use esp_idf_svc::hal::peripherals::Peripherals;
use esp_idf_svc::mqtt::client::{EspMqttClient, EventPayload, MqttClientConfiguration, QoS};
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use esp_idf_svc::wifi::{AuthMethod, BlockingWifi, ClientConfiguration, Configuration, EspWifi};

// --- Config loaded from cfg.toml at build time (see cfg.toml.example) ---
#[toml_cfg::toml_config]
pub struct Config {
    #[default("")]
    wifi_ssid: &'static str,
    #[default("")]
    wifi_pass: &'static str,
    #[default("mqtt://broker.example.com:1883")]
    mqtt_url: &'static str,
    #[default("esp32-relay")]
    mqtt_client_id: &'static str,
    #[default("esp32/relay")]
    topic_relay_prefix: &'static str,
    #[default("esp32/relay_status")]
    topic_status: &'static str,
}

// --- Firmware behavior constants ---
const RELAY_COUNT: usize = 4;
/// Most ESP32 relay boards drive the relay transistor directly (GPIO high = relay energized).
/// Set to false for opto-isolated boards whose relays energize on GPIO low.
const RELAY_ACTIVE_HIGH: bool = true;
const HEARTBEAT_SECS: u64 = 30;
const SUBSCRIBE_RETRY_MS: u64 = 500;
const WIFI_CHECK_SECS: u64 = 5;
/// `connect()`/`wait_netif_up()` log errors with Debug formatting; the default pthread stack
/// (3 KiB) is too tight for that.
const WIFI_WATCHDOG_STACK: usize = 8 * 1024;

#[derive(Clone, Copy, Debug)]
enum Action {
    On,
    Off,
    Toggle,
}

impl Action {
    fn parse(text: &str) -> Option<Self> {
        match text.trim() {
            "on" => Some(Self::On),
            "off" => Some(Self::Off),
            "toggle" => Some(Self::Toggle),
            _ => None,
        }
    }

    fn reason(self) -> &'static str {
        match self {
            Self::On => "switch_on",
            Self::Off => "switch_off",
            Self::Toggle => "switch_toggle",
        }
    }
}

/// A validated command for one relay. `relay` is the 1-based number used in the MQTT topic.
struct Command {
    relay: u8,
    action: Action,
}

/// One status publish. `relays` is a bitmask snapshot (bit 0 = relay 1); `relay` is the relay the
/// event was about, if any (None for heartbeat/connected).
struct Status {
    relays: u8,
    relay: Option<u8>,
    reason: &'static str,
}

/// Parses the relay number out of `<prefix>/<n>`. Returns Err with the raw suffix if the topic is
/// under the prefix but `n` isn't 1..=RELAY_COUNT, so the caller can report the rejection.
fn parse_relay_topic<'a>(prefix: &str, topic: &'a str) -> Option<Result<u8, &'a str>> {
    let suffix = topic.strip_prefix(prefix)?.strip_prefix('/')?;
    Some(match suffix.parse::<u8>() {
        Ok(n) if (1..=RELAY_COUNT as u8).contains(&n) => Ok(n),
        _ => Err(suffix),
    })
}

fn main() -> anyhow::Result<()> {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    let app_config = CONFIG;

    if app_config.wifi_ssid.is_empty() {
        anyhow::bail!(
            "wifi_ssid is empty — fill in cfg.toml (see cfg.toml.example) before building"
        );
    }

    let peripherals = Peripherals::take()?;

    // --- Relays: driven to "off" before anything else (WiFi can take seconds) so a load never
    // floats in an undefined state during boot. Relay N is `relays[N - 1]`.
    // TODO: these pins are placeholders (the common "ESP32 Relay X4" layout) — confirm against
    // the actual board's silkscreen/schematic before connecting real loads. ---
    let mut relays: [PinDriver<'_, Output>; RELAY_COUNT] = [
        PinDriver::output(peripherals.pins.gpio32)?,
        PinDriver::output(peripherals.pins.gpio33)?,
        PinDriver::output(peripherals.pins.gpio25)?,
        PinDriver::output(peripherals.pins.gpio26)?,
    ];
    for pin in relays.iter_mut() {
        set_relay(pin, false)?;
    }

    let sys_loop = EspSystemEventLoop::take()?;
    let nvs = EspDefaultNvsPartition::take()?;

    // --- WiFi setup ---
    let mut wifi = BlockingWifi::wrap(
        EspWifi::new(peripherals.modem, sys_loop.clone(), Some(nvs))?,
        sys_loop,
    )?;

    wifi.set_configuration(&Configuration::Client(ClientConfiguration {
        ssid: app_config.wifi_ssid.try_into().unwrap(),
        password: app_config.wifi_pass.try_into().unwrap(),
        auth_method: AuthMethod::WPA2Personal,
        ..Default::default()
    }))?;

    wifi.start()?;
    match connect_wifi(&mut wifi) {
        Ok(()) => log::info!("WiFi connected"),
        Err(e) => log::warn!("WiFi connect failed at boot: {e:?}, watchdog will retry"),
    }

    // --- WiFi watchdog thread: owns `wifi` from here on. esp-idf-svc doesn't reconnect the
    // station on its own, so after a router reboot or AP dropout this retries every
    // WIFI_CHECK_SECS until the interface is back up. Relay state is untouched. The MQTT client
    // reconnects by itself once the network returns, and the subscriber thread resubscribes. ---
    thread::Builder::new()
        .stack_size(WIFI_WATCHDOG_STACK)
        .spawn(move || {
            let mut down = false;
            loop {
                if !wifi.is_up().unwrap_or(false) {
                    if !down {
                        log::warn!("WiFi down, reconnecting...");
                        down = true;
                    }
                    match connect_wifi(&mut wifi) {
                        Ok(()) => {
                            log::info!("WiFi reconnected");
                            down = false;
                        }
                        Err(e) => log::warn!("WiFi reconnect failed: {e:?}"),
                    }
                }
                thread::sleep(Duration::from_secs(WIFI_CHECK_SECS));
            }
        })?;

    // --- Shared state ---
    // `relay_bits`: bitmask of energized relays (bit 0 = relay 1). Written only by the main
    // thread (which owns the pins); read by others for status snapshots.
    // `subscribed`: true while the relay-topic subscription is live. Cleared on every MQTT
    // Disconnected event so the subscriber thread resubscribes after the broker comes back
    // (the client uses a clean session, so the broker forgets subscriptions on disconnect).
    let relay_bits = Arc::new(AtomicU8::new(0));
    let subscribed = Arc::new(AtomicBool::new(false));

    // --- MQTT setup ---
    let mqtt_config = MqttClientConfiguration {
        client_id: Some(app_config.mqtt_client_id),
        ..Default::default()
    };

    let (client, mut connection) = EspMqttClient::new(app_config.mqtt_url, &mqtt_config)?;
    let client = Arc::new(Mutex::new(client));

    // Commands flow drainer -> main thread (which owns the pins). Status flows from every
    // producer -> the status-publisher thread, the only thread that calls `publish()`.
    let (cmd_tx, cmd_rx) = mpsc::channel::<Command>();
    let (status_tx, status_rx) = mpsc::channel::<Status>();

    // --- Connection-draining thread: must be pumping `connection.next()` continuously or the
    // client's internal connect/subscribe state machine never progresses (esp-rs/esp-idf-svc#441).
    // It must never call a blocking client method itself (`publish()` from here deadlocks), so
    // it only forwards over channels. ---
    {
        let prefix = app_config.topic_relay_prefix;
        let relay_bits = relay_bits.clone();
        let subscribed = subscribed.clone();
        let status_tx = status_tx.clone();
        thread::spawn(move || {
            while let Ok(event) = connection.next() {
                match event.payload() {
                    EventPayload::Received { topic, data, .. } => {
                        let Some(topic) = topic else { continue };
                        let reject = |relay: Option<u8>, reason| {
                            let _ = status_tx.send(Status {
                                relays: relay_bits.load(Ordering::Relaxed),
                                relay,
                                reason,
                            });
                        };
                        match parse_relay_topic(prefix, topic) {
                            None => log::warn!("Received message on unexpected topic: {topic}"),
                            Some(Err(suffix)) => {
                                log::warn!("Rejected unknown relay: {suffix}");
                                reject(None, "rejected_invalid_relay");
                            }
                            Some(Ok(relay)) => {
                                match std::str::from_utf8(data).ok().and_then(Action::parse) {
                                    Some(action) => {
                                        let _ = cmd_tx.send(Command { relay, action });
                                    }
                                    None => {
                                        log::warn!(
                                            "Rejected invalid payload for relay {relay}: {:?}",
                                            String::from_utf8_lossy(data)
                                        );
                                        reject(Some(relay), "rejected_invalid_command");
                                    }
                                }
                            }
                        }
                    }
                    EventPayload::Disconnected => {
                        log::warn!("MQTT disconnected");
                        subscribed.store(false, Ordering::Relaxed);
                    }
                    _ => {}
                }
            }
            log::warn!("MQTT connection closed");
        });
    }

    // --- Status-publisher thread (see channel comment above). ---
    {
        let client = client.clone();
        let topic_status = app_config.topic_status;
        thread::spawn(move || {
            while let Ok(status) = status_rx.recv() {
                publish_status(&client, topic_status, &status);
            }
        });
    }

    // --- Subscriber thread: whenever `subscribed` is false (boot, or after a disconnect), retry
    // subscribe() until it succeeds. Polls the flag rather than waiting for a Connected event —
    // gating subscribe on Connected has been observed to hang forever with esp-idf-svc. It must
    // run on a different thread than the one calling `connection.next()`. The first attempts
    // after boot/disconnect are expected to fail with ESP_FAIL until the connection is up. ---
    {
        let filter = format!("{}/+", app_config.topic_relay_prefix);
        let relay_bits = relay_bits.clone();
        let status_tx = status_tx.clone();
        thread::spawn(move || {
            let mut warned = false;
            loop {
                if !subscribed.load(Ordering::Relaxed) {
                    match client.lock().unwrap().subscribe(&filter, QoS::AtLeastOnce) {
                        Ok(_) => {
                            log::info!("Subscribed to {filter}");
                            subscribed.store(true, Ordering::Relaxed);
                            warned = false;
                            let _ = status_tx.send(Status {
                                relays: relay_bits.load(Ordering::Relaxed),
                                relay: None,
                                reason: "connected",
                            });
                        }
                        Err(e) if !warned => {
                            log::warn!("Failed to subscribe to {filter}: {e:?}, retrying...");
                            warned = true;
                        }
                        Err(_) => {}
                    }
                }
                thread::sleep(Duration::from_millis(SUBSCRIBE_RETRY_MS));
            }
        });
    }

    // --- Heartbeat thread: periodically republishes current state ---
    {
        let relay_bits = relay_bits.clone();
        let status_tx = status_tx.clone();
        thread::spawn(move || loop {
            thread::sleep(Duration::from_secs(HEARTBEAT_SECS));
            let _ = status_tx.send(Status {
                relays: relay_bits.load(Ordering::Relaxed),
                relay: None,
                reason: "heartbeat",
            });
        });
    }

    // --- Main loop: sole owner of the relay pins; applies commands as they arrive. ---
    while let Ok(Command { relay, action }) = cmd_rx.recv() {
        let bit = 1u8 << (relay - 1);
        let bits = relay_bits.load(Ordering::Relaxed);
        let on = match action {
            Action::On => true,
            Action::Off => false,
            Action::Toggle => bits & bit == 0,
        };
        set_relay(&mut relays[(relay - 1) as usize], on)?;
        let bits = if on { bits | bit } else { bits & !bit };
        relay_bits.store(bits, Ordering::Relaxed);
        log::info!("Relay {relay} -> {}", if on { "on" } else { "off" });
        let _ = status_tx.send(Status {
            relays: bits,
            relay: Some(relay),
            reason: action.reason(),
        });
    }

    // Unreachable in practice (the drainer thread holds `cmd_tx` forever).
    Ok(())
}

/// Brings the station up: associates with the AP if needed, then waits for an IP. Each step
/// times out after ~15s inside esp-idf-svc.
fn connect_wifi(wifi: &mut BlockingWifi<EspWifi<'static>>) -> anyhow::Result<()> {
    if !wifi.is_connected()? {
        wifi.connect()?;
    }
    wifi.wait_netif_up()?;
    Ok(())
}

fn set_relay(pin: &mut PinDriver<'_, Output>, on: bool) -> anyhow::Result<()> {
    if on == RELAY_ACTIVE_HIGH {
        pin.set_high()?;
    } else {
        pin.set_low()?;
    }
    Ok(())
}

/// Publishes a small JSON status payload, e.g.
/// {"relays":["on","off","off","off"],"relay":1,"reason":"switch_on"} or
/// {"relays":["on","off","off","off"],"relay":null,"reason":"heartbeat"}.
fn publish_status(client: &Mutex<EspMqttClient<'_>>, topic_status: &str, status: &Status) {
    let relays = (0..RELAY_COUNT)
        .map(|i| if status.relays & (1 << i) != 0 { r#""on""# } else { r#""off""# })
        .collect::<Vec<_>>()
        .join(",");
    let relay = status.relay.map_or("null".to_string(), |r| r.to_string());
    let payload = format!(
        r#"{{"relays":[{relays}],"relay":{relay},"reason":"{}"}}"#,
        status.reason
    );
    if let Err(e) =
        client
            .lock()
            .unwrap()
            .publish(topic_status, QoS::AtLeastOnce, false, payload.as_bytes())
    {
        log::warn!("Failed to publish status: {e:?}");
    }
}
