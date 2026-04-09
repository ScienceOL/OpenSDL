// OpenSDL Child Node Firmware — ESP32 Serial-to-MQTT Bridge
//
// This firmware turns an ESP32 into a transparent serial bridge:
//   1. Connect WiFi
//   2. Discover mother node via mDNS (_osdl._tcp.local)
//   3. Connect MQTT broker → publish registration
//   4. Subscribe osdl/serial/{node_id}/tx → write bytes to UART (RS-485)
//   5. UART receive → publish osdl/serial/{node_id}/rx
//
// The ESP32 has ZERO device knowledge. All protocol parsing happens on the
// mother node in Rust. This node is just a "network cable to serial port".
//
// Hardware: ESP32-S3 (~$3) + MAX485/SP3485 (~$1) + PCB
// Total cost per node: ~$5

#include <Arduino.h>
#include <WiFi.h>
#include <ESPmDNS.h>
#include <PubSubClient.h>
#include "config.h"

// MQTT topics (built from NODE_ID)
static char topic_register[64];
static char topic_heartbeat[64];
static char topic_tx[64];  // subscribe: mother → child
static char topic_rx[64];  // publish:   child → mother

// Discovered mother node address
static IPAddress mqtt_ip;
static uint16_t  mqtt_port = MQTT_PORT;

WiFiClient wifiClient;
PubSubClient mqtt(wifiClient);

// UART buffer for incoming serial data
#define UART_BUF_SIZE 256
static uint8_t uart_buf[UART_BUF_SIZE];
static size_t  uart_len = 0;

// Timing
static unsigned long last_heartbeat = 0;
static unsigned long last_uart_byte = 0;

// RS-485 direction control
static inline void rs485_tx_enable() {
#if RS485_DE_PIN >= 0
    digitalWrite(RS485_DE_PIN, HIGH);
#endif
}

static inline void rs485_rx_enable() {
#if RS485_DE_PIN >= 0
    digitalWrite(RS485_DE_PIN, LOW);
#endif
}

// ─── WiFi ────────────────────────────────────────────────────────────

void wifi_connect() {
    Serial.printf("[WiFi] Connecting to %s", WIFI_SSID);
    WiFi.mode(WIFI_STA);
    WiFi.begin(WIFI_SSID, WIFI_PASSWORD);

    int attempts = 0;
    while (WiFi.status() != WL_CONNECTED && attempts < 60) {
        delay(500);
        Serial.print(".");
        attempts++;
    }

    if (WiFi.status() == WL_CONNECTED) {
        Serial.printf("\n[WiFi] Connected: %s\n", WiFi.localIP().toString().c_str());
    } else {
        Serial.println("\n[WiFi] FAILED — restarting in 5s");
        delay(5000);
        ESP.restart();
    }
}

// ─── mDNS Discovery ─────────────────────────────────────────────────

// Discover the mother node by querying for _osdl._tcp.local mDNS service.
// Returns true if found, populating mqtt_ip and mqtt_port.
bool discover_mother() {
    // If a static host is configured, use it directly
    const char* static_host = MQTT_HOST;
    if (static_host[0] != '\0') {
        mqtt_ip.fromString(static_host);
        mqtt_port = MQTT_PORT;
        Serial.printf("[mDNS] Using static host: %s:%d\n",
                      mqtt_ip.toString().c_str(), mqtt_port);
        return true;
    }

    Serial.println("[mDNS] Searching for _osdl._tcp.local ...");

    if (!MDNS.begin(NODE_ID)) {
        Serial.println("[mDNS] Failed to start mDNS client");
        return false;
    }

    // Query for OpenSDL service
    int n = MDNS.queryService("osdl", "tcp");

    if (n > 0) {
        // Use the first result
        mqtt_ip   = MDNS.IP(0);
        mqtt_port = MDNS.port(0);
        Serial.printf("[mDNS] Found mother node: %s:%d (%s)\n",
                      mqtt_ip.toString().c_str(),
                      mqtt_port,
                      MDNS.hostname(0).c_str());
        return true;
    }

    Serial.println("[mDNS] No mother node found");
    return false;
}

// ─── MQTT ────────────────────────────────────────────────────────────

// Called when a message arrives on osdl/serial/{node_id}/tx
void mqtt_callback(char* topic, byte* payload, unsigned int length) {
    // Write received bytes directly to UART (→ device via RS-485)
    rs485_tx_enable();
    Serial1.write(payload, length);
    Serial1.flush();  // wait until TX complete
    rs485_rx_enable();

    Serial.printf("[TX] %u bytes → UART\n", length);
}

void mqtt_connect() {
    mqtt.setServer(mqtt_ip, mqtt_port);
    mqtt.setCallback(mqtt_callback);
    mqtt.setBufferSize(512);  // enough for serial frames

    while (!mqtt.connected()) {
        Serial.printf("[MQTT] Connecting to %s:%d as %s...\n",
                      mqtt_ip.toString().c_str(), mqtt_port, NODE_ID);

        if (mqtt.connect(NODE_ID)) {
            Serial.println("[MQTT] Connected");

            // Subscribe to TX topic (mother → child serial bytes)
            mqtt.subscribe(topic_tx);
            Serial.printf("[MQTT] Subscribed: %s\n", topic_tx);

            // Publish registration
            char reg_payload[256];
            snprintf(reg_payload, sizeof(reg_payload),
                     "{\"hardware_id\":\"%s\",\"baud_rate\":%d}",
                     HARDWARE_ID, DEVICE_BAUD);
            mqtt.publish(topic_register, reg_payload, true);  // retained
            Serial.printf("[MQTT] Registered: %s\n", reg_payload);
        } else {
            Serial.printf("[MQTT] Failed (rc=%d), retry in 3s\n", mqtt.state());
            delay(3000);

            // Re-discover in case mother node IP changed
            discover_mother();
            mqtt.setServer(mqtt_ip, mqtt_port);
        }
    }
}

// ─── Serial (UART1 → RS-485 → Device) ───────────────────────────────

// Flush accumulated UART bytes as a single MQTT message.
// We wait for a gap in serial data (inter-frame silence) to detect
// the end of a response frame, since protocols use varying frame lengths.
#define UART_SILENCE_MS 50  // 50ms silence = end of frame at 9600 baud

void uart_flush_to_mqtt() {
    if (uart_len == 0) return;

    mqtt.publish(topic_rx, uart_buf, uart_len);
    Serial.printf("[RX] %u bytes → MQTT\n", uart_len);
    uart_len = 0;
}

void uart_poll() {
    while (Serial1.available()) {
        if (uart_len < UART_BUF_SIZE) {
            uart_buf[uart_len++] = Serial1.read();
        } else {
            Serial1.read();  // discard overflow
        }
        last_uart_byte = millis();
    }

    // If we have data and silence exceeded threshold, flush
    if (uart_len > 0 && (millis() - last_uart_byte) >= UART_SILENCE_MS) {
        uart_flush_to_mqtt();
    }
}

// ─── Heartbeat ───────────────────────────────────────────────────────

void heartbeat() {
    if (millis() - last_heartbeat >= HEARTBEAT_INTERVAL_MS) {
        mqtt.publish(topic_heartbeat, "1");
        last_heartbeat = millis();
    }
}

// ─── Setup & Loop ────────────────────────────────────────────────────

void setup() {
    // Debug console
    Serial.begin(115200);
    delay(100);
    Serial.println("\n[OSDL] Child node starting...");
    Serial.printf("[OSDL] Node: %s  Hardware: %s\n", NODE_ID, HARDWARE_ID);

    // Build MQTT topic strings
    snprintf(topic_register,  sizeof(topic_register),  "osdl/nodes/%s/register",  NODE_ID);
    snprintf(topic_heartbeat, sizeof(topic_heartbeat), "osdl/nodes/%s/heartbeat", NODE_ID);
    snprintf(topic_tx,        sizeof(topic_tx),        "osdl/serial/%s/tx",       NODE_ID);
    snprintf(topic_rx,        sizeof(topic_rx),        "osdl/serial/%s/rx",       NODE_ID);

    // RS-485 direction control pin
#if RS485_DE_PIN >= 0
    pinMode(RS485_DE_PIN, OUTPUT);
    rs485_rx_enable();  // default to receive mode
#endif

    // Device UART (Serial1) on configurable pins
    Serial1.begin(DEVICE_BAUD, SERIAL_8N1, UART_RX_PIN, UART_TX_PIN);

    // Connect WiFi
    wifi_connect();

    // Discover mother node via mDNS
    while (!discover_mother()) {
        Serial.printf("[mDNS] Retrying in %d ms...\n", MDNS_TIMEOUT_MS);
        delay(MDNS_TIMEOUT_MS);
    }

    // Connect MQTT
    mqtt_connect();
}

void loop() {
    // Reconnect if needed
    if (WiFi.status() != WL_CONNECTED) {
        wifi_connect();
    }
    if (!mqtt.connected()) {
        mqtt_connect();
    }

    // Process MQTT messages (calls mqtt_callback for TX)
    mqtt.loop();

    // Poll UART for device responses
    uart_poll();

    // Periodic heartbeat
    heartbeat();
}
