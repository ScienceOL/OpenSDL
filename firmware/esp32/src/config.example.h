// Copy this file to config.h and fill in your values.

#ifndef OSDL_CONFIG_H
#define OSDL_CONFIG_H

// WiFi
#define WIFI_SSID     "YourWiFi"
#define WIFI_PASSWORD "YourPassword"

// MQTT broker discovery
// The child node will auto-discover the mother node via mDNS (_osdl._tcp.local).
// Set MQTT_HOST to "" to enable auto-discovery (recommended).
// Set a specific IP to skip mDNS and connect directly (fallback).
#define MQTT_HOST     ""        // "" = auto-discover via mDNS
#define MQTT_PORT     1883      // only used when MQTT_HOST is set

// Node identity — unique per physical child node
#define NODE_ID       "pump-01"

// Hardware ID — must match a device_type in the registry YAML
// Examples:
//   "syringe_pump_with_valve.runze.SY03B-T06"
//   "syringe_pump_with_valve.runze.SY03B-T08"
//   "heater_stirrer_dalong"
#define HARDWARE_ID   "syringe_pump_with_valve.runze.SY03B-T06"

// Serial (UART → device)
#define DEVICE_BAUD   9600
#define UART_TX_PIN   17    // GPIO connected to RS-485 TX (DI)
#define UART_RX_PIN   18    // GPIO connected to RS-485 RX (RO)
#define RS485_DE_PIN  16    // GPIO connected to RS-485 DE/RE (set -1 if not used)

// Heartbeat interval (ms)
#define HEARTBEAT_INTERVAL_MS  10000

// mDNS discovery timeout (ms) — how long to search before retrying
#define MDNS_TIMEOUT_MS  5000

#endif
