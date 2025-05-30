// src/networking.rs
/// Network communication and device discovery
use crate::types::{AppError, ConnectionStatus, Device, Message};
use crate::clipboard::{calculate_hash, ClipboardManager, HashTracker};
use if_addrs::{get_if_addrs, IfAddr};
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio::sync::mpsc;
use format;

/// Shared network state
#[derive(Clone)]
pub struct NetworkState {
    pub devices: Arc<Mutex<HashMap<IpAddr, Device>>>,
    pub connections: Arc<Mutex<HashMap<IpAddr, mpsc::UnboundedSender<String>>>>,
    pub hash_tracker: HashTracker,
}

impl NetworkState {
    pub fn new() -> Self {
        Self {
            devices: Arc::new(Mutex::new(HashMap::new())),
            connections: Arc::new(Mutex::new(HashMap::new())),
            hash_tracker: HashTracker::new(10),
        }
    }
}

/// Network communication channels
pub struct NetworkChannels {
    pub pair_request_tx: mpsc::UnboundedSender<(IpAddr, bool)>,
    pub disconnect_tx: mpsc::UnboundedSender<IpAddr>,
    pub clipboard_send_tx: mpsc::UnboundedSender<String>,
}

/// Network manager for all communication tasks
pub struct NetworkManager;

impl NetworkManager {
    /// Initialize networking with proper error handling
    pub async fn start_networking(
        device_id: String,
        port: u16,
        state: NetworkState,
        pair_request_rx: mpsc::UnboundedReceiver<(IpAddr, bool)>,
        disconnect_rx: mpsc::UnboundedReceiver<IpAddr>,
        gui_clipboard_rx: mpsc::UnboundedReceiver<String>,
    ) -> Result<(), AppError> {
        // Verify ports are available
        Self::check_ports_available(port).await?;

        // Create internal clipboard channel
        let (clipboard_send_tx, clipboard_send_rx) = mpsc::unbounded_channel();

        // Start all networking tasks
        tokio::spawn(Self::discovery_task(device_id.clone(), port, state.clone()));
        tokio::spawn(Self::tcp_server_task(port, device_id.clone(), state.clone()));
        tokio::spawn(Self::clipboard_monitor_task(state.clone(), clipboard_send_tx));
        tokio::spawn(Self::connection_manager_task(
            device_id, port, state.clone(), pair_request_rx, disconnect_rx,
            clipboard_send_rx, gui_clipboard_rx
        ));
        tokio::spawn(Self::cleanup_task(state));

        // Keep networking alive
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }

    /// Check if required ports are available
    async fn check_ports_available(port: u16) -> Result<(), AppError> {
        let udp_addr = format!("0.0.0.0:{}", port);
        let tcp_addr = format!("0.0.0.0:{}", port + 1);

        let _udp_socket = UdpSocket::bind(&udp_addr).await
            .map_err(|_| AppError::PortInUse(port))?;
        let _tcp_listener = TcpListener::bind(&tcp_addr).await
            .map_err(|_| AppError::PortInUse(port + 1))?;

        Ok(())
    }

    /// Get broadcast addresses for all active network interfaces
    fn get_broadcast_addresses() -> Vec<IpAddr> {
        let mut broadcast_addrs = Vec::new();

        if let Ok(interfaces) = get_if_addrs() {
            for interface in interfaces {
                if interface.is_loopback() {
                    continue;
                }

                if let IfAddr::V4(ipv4) = interface.addr {
                    let ip = ipv4.ip;
                    let netmask = ipv4.netmask;
                    let ip_u32 = u32::from(ip);
                    let netmask_u32 = u32::from(netmask);
                    let broadcast_u32 = ip_u32 | !netmask_u32;
                    let broadcast_ip = Ipv4Addr::from(broadcast_u32);
                    broadcast_addrs.push(IpAddr::V4(broadcast_ip));

                    log::debug!("Interface {}: {} -> broadcast {}", interface.name, ip, broadcast_ip);
                }
            }
        }

        if broadcast_addrs.is_empty() {
            broadcast_addrs.push(IpAddr::V4(Ipv4Addr::new(255, 255, 255, 255)));
            log::debug!("No interfaces found, using global broadcast");
        }

        broadcast_addrs
    }

    /// UDP discovery and message handling
    async fn discovery_task(device_id: String, port: u16, state: NetworkState) {
        let socket = match UdpSocket::bind(format!("0.0.0.0:{}", port)).await {
            Ok(socket) => socket,
            Err(e) => {
                log::error!("Failed to bind UDP socket: {}", e);
                return;
            }
        };

        if let Err(e) = socket.set_broadcast(true) {
            log::error!("Failed to set broadcast: {}", e);
            return;
        }

        let discovery_msg = Message::Discovery {
            device_id: device_id.clone(),
            port,
        };

        let mut interval = tokio::time::interval(Duration::from_secs(1));
        let mut buf = [0; 1024];

        loop {
            tokio::select! {
                _ = interval.tick() => {
                    if let Ok(json) = serde_json::to_string(&discovery_msg) {
                        let broadcast_addrs = Self::get_broadcast_addresses();
                        for addr in broadcast_addrs {
                            let target = SocketAddr::new(addr, port);
                            let _ = socket.send_to(json.as_bytes(), target).await;
                        }
                    }
                }

                result = socket.recv_from(&mut buf) => {
                    if let Ok((len, addr)) = result {
                        if let Ok(json_str) = std::str::from_utf8(&buf[..len]) {
                            if let Ok(message) = serde_json::from_str::<Message>(json_str) {
                                Self::handle_discovery_message(message, addr.ip(), &device_id, &state).await;
                            }
                        }
                    }
                }
            }
        }
    }

    /// Handle incoming discovery messages
    async fn handle_discovery_message(message: Message, sender_ip: IpAddr, device_id: &str, state: &NetworkState) {
        match message {
            Message::Discovery { device_id: peer_id, .. } => {
                if peer_id != device_id {
                    let mut devices = state.devices.lock().unwrap();
                    let device = devices.entry(sender_ip).or_insert_with(|| Device {
                        ip: sender_ip,
                        device_id: peer_id,
                        last_seen: Instant::now(),
                        connection_status: ConnectionStatus::Disconnected,
                    });
                    device.last_seen = Instant::now();
                }
            }
            Message::PairRequest { device_id: peer_id } => {
                if peer_id != device_id {
                    let mut devices = state.devices.lock().unwrap();
                    if let Some(device) = devices.get_mut(&sender_ip) {
                        if device.connection_status == ConnectionStatus::Disconnected {
                            device.connection_status = ConnectionStatus::Connecting;
                        }
                    }
                }
            }
            Message::PairConfirm { device_id: peer_id } => {
                if peer_id != device_id {
                    // Initiate TCP connection
                    // This will be handled by connection manager
                }
            }
            _ => {}
        }
    }

    /// TCP server for incoming connections
    async fn tcp_server_task(port: u16, device_id: String, state: NetworkState) {
        let tcp_port = port + 1;
        let listener = match TcpListener::bind(format!("0.0.0.0:{}", tcp_port)).await {
            Ok(listener) => listener,
            Err(e) => {
                log::error!("Failed to bind TCP listener: {}", e);
                return;
            }
        };

        log::info!("TCP server listening on port {}", tcp_port);

        while let Ok((stream, addr)) = listener.accept().await {
            let peer_ip = addr.ip();
            let device_id = device_id.clone();
            let state = state.clone();

            tokio::spawn(async move {
                Self::handle_tcp_connection(stream, peer_ip, device_id, state).await;
            });
        }
    }

    /// Handle individual TCP connection
    async fn handle_tcp_connection(stream: TcpStream, peer_ip: IpAddr, device_id: String, state: NetworkState) {
        // Update connection status
        {
            let mut devices = state.devices.lock().unwrap();
            if let Some(device) = devices.get_mut(&peer_ip) {
                device.connection_status = ConnectionStatus::Connected;
            }
        }

        let (read_half, write_half) = stream.into_split();
        let (tx, mut rx) = mpsc::unbounded_channel::<String>();

        state.connections.lock().unwrap().insert(peer_ip, tx);

        // Handle outgoing messages
        let write_task = {
            let device_id = device_id.clone();
            tokio::spawn(async move {
                let mut write_half = write_half;
                while let Some(content) = rx.recv().await {
                    if content.len() > 50 * 1024 * 1024 { // 50MB limit
                        continue;
                    }

                    let message = Message::ClipboardData {
                        device_id: device_id.clone(),
                        content,
                    };
                    if let Ok(json) = serde_json::to_string(&message) {
                        let json_line = format!("{}\n", json);
                        if write_half.write_all(json_line.as_bytes()).await.is_err() {
                            break;
                        }
                    }
                }
            })
        };

        // Handle incoming messages
        let read_task = {
            let state_clone = state.clone();
            tokio::spawn(async move {
                let reader = BufReader::new(read_half);
                let mut lines = reader.lines();

                while let Ok(Some(line)) = lines.next_line().await {
                    if let Ok(Message::ClipboardData { content, .. }) = serde_json::from_str(&line) {
                        if content.len() > 50 * 1024 * 1024 {
                            continue;
                        }

                        let content_hash = calculate_hash(&content);
                        if !state_clone.hash_tracker.contains(content_hash) {
                            state_clone.hash_tracker.insert(content_hash);

                            let mut clipboard = ClipboardManager::new();
                            if let Err(e) = clipboard.set_text(&content) {
                                log::error!("Failed to set clipboard: {}", e);
                            }

                            tokio::time::sleep(Duration::from_millis(50)).await;
                        }
                    }
                }
            })
        };

        tokio::select! {
            _ = write_task => {},
            _ = read_task => {},
        }

        // Clean up
        state.connections.lock().unwrap().remove(&peer_ip);
        {
            let mut devices = state.devices.lock().unwrap();
            if let Some(device) = devices.get_mut(&peer_ip) {
                device.connection_status = ConnectionStatus::Reconnecting;
            }
        }
    }

    /// Monitor clipboard changes
    async fn clipboard_monitor_task(
        state: NetworkState,
        clipboard_send_tx: mpsc::UnboundedSender<String>,
    ) {
        let mut clipboard = ClipboardManager::new();
        let mut last_content_hash = 0u64;
        let mut interval = tokio::time::interval(Duration::from_millis(250));

        loop {
            interval.tick().await;

            if let Ok(content) = clipboard.get_text() {
                let content_hash = calculate_hash(&content);

                if content_hash != last_content_hash {
                    if !state.hash_tracker.contains(content_hash) {
                        state.hash_tracker.insert(content_hash);
                        last_content_hash = content_hash;
                        let _ = clipboard_send_tx.send(content);
                    } else {
                        last_content_hash = content_hash;
                    }
                }
            }
        }
    }

    /// Manage connections and handle requests
    async fn connection_manager_task(
        device_id: String,
        port: u16,
        state: NetworkState,
        mut pair_request_rx: mpsc::UnboundedReceiver<(IpAddr, bool)>,
        mut disconnect_rx: mpsc::UnboundedReceiver<IpAddr>,
        mut clipboard_rx: mpsc::UnboundedReceiver<String>,
        mut gui_clipboard_rx: mpsc::UnboundedReceiver<String>,
    ) {
        let mut reconnect_interval = tokio::time::interval(Duration::from_secs(3));

        loop {
            tokio::select! {
                Some((peer_ip, is_confirm)) = pair_request_rx.recv() => {
                    if is_confirm {
                        Self::send_pair_message(peer_ip, port, Message::PairConfirm { device_id: device_id.clone() }).await;
                        Self::attempt_tcp_connection(peer_ip, port, device_id.clone(), state.clone()).await;
                    } else {
                        Self::send_pair_message(peer_ip, port, Message::PairRequest { device_id: device_id.clone() }).await;
                    }
                }

                Some(peer_ip) = disconnect_rx.recv() => {
                    state.connections.lock().unwrap().remove(&peer_ip);
                    let mut devices = state.devices.lock().unwrap();
                    if let Some(device) = devices.get_mut(&peer_ip) {
                        device.connection_status = ConnectionStatus::Disconnected;
                    }
                }

                Some(content) = clipboard_rx.recv() => {
                    Self::broadcast_clipboard_content(&content, &state).await;
                }

                Some(content) = gui_clipboard_rx.recv() => {
                    Self::broadcast_clipboard_content(&content, &state).await;
                }

                _ = reconnect_interval.tick() => {
                    Self::handle_reconnections(port, &device_id, &state).await;
                }
            }
        }
    }

    /// Send UDP pairing message
    async fn send_pair_message(peer_ip: IpAddr, port: u16, message: Message) {
        if let Ok(socket) = UdpSocket::bind("0.0.0.0:0").await {
            if let Ok(json) = serde_json::to_string(&message) {
                let target = SocketAddr::new(peer_ip, port);
                let _ = socket.send_to(json.as_bytes(), target).await;
            }
        }
    }

    /// Attempt TCP connection
    async fn attempt_tcp_connection(peer_ip: IpAddr, port: u16, device_id: String, state: NetworkState) {
        let tcp_port = port + 1;
        let tcp_addr = SocketAddr::new(peer_ip, tcp_port);

        if let Ok(stream) = TcpStream::connect(tcp_addr).await {
            tokio::spawn(async move {
                Self::handle_tcp_connection(stream, peer_ip, device_id, state).await;
            });
        }
    }

    /// Broadcast clipboard content to all connections
    async fn broadcast_clipboard_content(content: &str, state: &NetworkState) {
        let connections = state.connections.lock().unwrap();
        for tx in connections.values() {
            let _ = tx.send(content.to_string());
        }
    }

    /// Handle automatic reconnections
    async fn handle_reconnections(port: u16, device_id: &str, state: &NetworkState) {
        let reconnecting_devices: Vec<IpAddr> = {
            let devices = state.devices.lock().unwrap();
            devices.iter()
                .filter(|(_, device)| device.connection_status == ConnectionStatus::Reconnecting)
                .map(|(ip, _)| *ip)
                .collect()
        };

        for ip in reconnecting_devices {
            Self::attempt_tcp_connection(ip, port, device_id.to_string(), state.clone()).await;
        }
    }

    /// Clean up old devices
    async fn cleanup_task(state: NetworkState) {
        let mut interval = tokio::time::interval(Duration::from_secs(5));

        loop {
            interval.tick().await;
            let now = Instant::now();

            let mut devices = state.devices.lock().unwrap();
            devices.retain(|_, device| {
                now.duration_since(device.last_seen) < Duration::from_secs(10)
            });
        }
    }
}
