/// Shared types and message definitions for ClipSync-RS
use serde::{Deserialize, Serialize};
use std::net::IpAddr;
use std::time::Instant;

/// Message types for network communication
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Message {
    /// Discovery announcement broadcast
    Discovery { device_id: String, port: u16 },
    /// Request to pair with a device
    PairRequest { device_id: String },
    /// Confirmation to accept pairing
    PairConfirm { device_id: String },
    /// Clipboard data synchronization
    ClipboardData { device_id: String, content: String },
    /// Intentional disconnect notification
    Disconnect { device_id: String },
}

/// Represents a discovered device on the network
#[derive(Debug, Clone)]
pub struct Device {
    pub ip: IpAddr,
    pub device_id: String,
    pub last_seen: Instant,
    pub connection_status: ConnectionStatus,
    pub intentionally_disconnected: bool,
}

/// Connection status for each device
#[derive(Debug, Clone, PartialEq)]
pub enum ConnectionStatus {
    Disconnected,    // Default state - can initiate connection
    Connecting,      // Remote device wants to connect - waiting for local confirmation
    Connected,       // TCP connection established
    Reconnecting,    // Lost connection - trying to (re)connect
}

/// Network status updates from networking layer to GUI
#[derive(Debug, Clone)]
pub enum NetworkStatus {
    Started(String),
    Error(String),
    StatusUpdate(String),
}

impl Device {
    pub fn new(ip: IpAddr, device_id: String) -> Self {
        Self {
            ip,
            device_id,
            last_seen: Instant::now(),
            connection_status: ConnectionStatus::Disconnected,
            intentionally_disconnected: false,
        }
    }
}
