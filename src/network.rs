/// Network operations and tasks for ClipSync-RS
use crate::types::{Device, Message, ConnectionStatus, NetworkStatus};
use crate::clipboard::ClipboardManager;
use if_addrs::{get_if_addrs, IfAddr};
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio::sync::mpsc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

/// Circular buffer for tracking recent clipboard hashes
#[derive(Clone)]
pub struct ClipboardHashTracker {
    hashes: Arc<Mutex<Vec<u64>>>,
    max_size: usize,
}

impl ClipboardHashTracker {
    pub fn new(max_size: usize) -> Self {
        Self {
            hashes: Arc::new(Mutex::new(Vec::with_capacity(max_size))),
            max_size,
        }
    }

    pub fn contains(&self, hash: u64) -> bool {
        self.hashes.lock().unwrap().contains(&hash)
    }

    pub fn insert(&self, hash: u64) {
        let mut hashes = self.hashes.lock().unwrap();
        if hashes.len() >= self.max_size {
            hashes.remove(0);
        }
        hashes.push(hash);
    }
}

/// Shared state between network threads
#[derive(Clone)]
pub struct NetworkState {
    pub devices: Arc<Mutex<HashMap<IpAddr, Device>>>,
    pub connections: Arc<Mutex<HashMap<IpAddr, mpsc::UnboundedSender<String>>>>,
    pub connection_shutdowns: Arc<Mutex<HashMap<IpAddr, Arc<AtomicBool>>>>,
    pub hash_tracker: ClipboardHashTracker,
}

impl NetworkState {
    pub fn new() -> Self {
        Self {
            devices: Arc::new(Mutex::new(HashMap::new())),
            connections: Arc::new(Mutex::new(HashMap::new())),
            connection_shutdowns: Arc::new(Mutex::new(HashMap::new())),
            hash_tracker: ClipboardHashTracker::new(10),
        }
    }
}

/// Application errors
#[derive(Debug)]
pub enum NetworkError {
    PortInUse(u16),
    BindError(String),
}

pub struct NetworkManager;

impl NetworkManager {
    /// Check if ports are available
    pub async fn check_ports_available(port: u16) -> Result<(), NetworkError> {
        let udp_addr = format!("0.0.0.0:{}", port);
        let tcp_addr = format!("0.0.0.0:{}", port + 1);

        let _udp_socket = UdpSocket::bind(&udp_addr).await
            .map_err(|_| NetworkError::PortInUse(port))?;
        let _tcp_listener = TcpListener::bind(&tcp_addr).await
            .map_err(|_| NetworkError::PortInUse(port + 1))?;

        Ok(())
    }

    /// Set up all networking tasks
    pub async fn setup_networking(
        device_id: String,
        port: u16,
        state: NetworkState,
        pair_request_rx: mpsc::UnboundedReceiver<(IpAddr, bool)>,
        disconnect_rx: mpsc::UnboundedReceiver<IpAddr>,
        gui_clipboard_rx: mpsc::UnboundedReceiver<String>,
        status_tx: mpsc::UnboundedSender<NetworkStatus>,
        shutdown_flag: Arc<AtomicBool>,
    ) -> Result<(), NetworkError> {
        // Check ports availability
        Self::check_ports_available(port).await?;

        // Send success status
        let _ = status_tx.send(NetworkStatus::Started(format!("Broadcasting on port {}", port)));

        // Create internal clipboard channel
        let (clipboard_send_tx, clipboard_send_rx) = mpsc::unbounded_channel();

        // Start all networking tasks
        let discovery_handle = tokio::spawn(Self::discovery_task(
            device_id.clone(),
            port,
            state.clone(),
            shutdown_flag.clone()
        ));

        let tcp_server_handle = tokio::spawn(Self::tcp_server_task(
            port,
            device_id.clone(),
            state.clone(),
            shutdown_flag.clone()
        ));

        let clipboard_monitor_handle = tokio::spawn(Self::clipboard_monitor_task(
            device_id.clone(),
            state.clone(),
            clipboard_send_tx,
            shutdown_flag.clone()
        ));

        let connection_manager_handle = tokio::spawn(Self::connection_manager_task(
            device_id,
            port,
            state.clone(),
            pair_request_rx,
            disconnect_rx,
            clipboard_send_rx,
            gui_clipboard_rx,
            shutdown_flag.clone()
        ));

        let cleanup_handle = tokio::spawn(Self::cleanup_task(
            state,
            shutdown_flag.clone()
        ));

        // Wait for shutdown signal or any task to complete
        tokio::select! {
            _ = discovery_handle => {},
            _ = tcp_server_handle => {},
            _ = clipboard_monitor_handle => {},
            _ = connection_manager_handle => {},
            _ = cleanup_handle => {},
        }

        Ok(())
    }

    /// Get all broadcast addresses for network interfaces
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
                }
            }
        }

        if broadcast_addrs.is_empty() {
            broadcast_addrs.push(IpAddr::V4(Ipv4Addr::new(255, 255, 255, 255)));
        }

        broadcast_addrs
    }

    /// UDP discovery task
    async fn discovery_task(
        device_id: String,
        port: u16,
        state: NetworkState,
        shutdown_flag: Arc<AtomicBool>
    ) {
        let socket = match UdpSocket::bind(format!("0.0.0.0:{}", port)).await {
            Ok(socket) => socket,
            Err(e) => {
                eprintln!("Failed to bind UDP socket: {}", e);
                return;
            }
        };

        if let Err(e) = socket.set_broadcast(true) {
            eprintln!("Failed to set broadcast: {}", e);
            return;
        }

        let discovery_msg = Message::Discovery {
            device_id: device_id.clone(),
            port,
        };

        let mut interval = tokio::time::interval(Duration::from_secs(1));
        let mut buf = [0; 1024];

        loop {
            if shutdown_flag.load(Ordering::Relaxed) {
                break;
            }

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
                                match message {
                                    Message::Discovery { device_id: peer_id, .. } => {
                                        if peer_id != device_id {
                                            let mut devices = state.devices.lock().unwrap();
                                            let device = devices.entry(addr.ip()).or_insert_with(|| {
                                                Device::new(addr.ip(), peer_id)
                                            });
                                            device.last_seen = Instant::now();
                                        }
                                    }
                                    Message::PairRequest { device_id: peer_id } => {
                                        if peer_id != device_id {
                                            let mut devices = state.devices.lock().unwrap();
                                            if let Some(device) = devices.get_mut(&addr.ip()) {
                                                if device.connection_status == ConnectionStatus::Disconnected {
                                                    device.connection_status = ConnectionStatus::Connecting;
                                                    device.intentionally_disconnected = false;
                                                }
                                            }
                                        }
                                    }
                                    Message::PairConfirm { device_id: peer_id } => {
                                        if peer_id != device_id {
                                            let tcp_port = port + 1;
                                            let tcp_addr = SocketAddr::new(addr.ip(), tcp_port);
                                            if let Ok(stream) = TcpStream::connect(tcp_addr).await {
                                                let state_clone = state.clone();
                                                let device_id_clone = device_id.clone();
                                                let connection_shutdown = Arc::new(AtomicBool::new(false));

                                                // Store shutdown signal for this connection
                                                state.connection_shutdowns.lock().unwrap().insert(addr.ip(), connection_shutdown.clone());

                                                tokio::spawn(async move {
                                                    Self::handle_tcp_connection(stream, addr.ip(), device_id_clone, state_clone, connection_shutdown).await;
                                                });
                                            }
                                        }
                                    }
                                    Message::Disconnect { device_id: peer_id } => {
                                        if peer_id != device_id {
                                            // Remote device is disconnecting from us
                                            Self::handle_remote_disconnect(addr.ip(), &state).await;
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// TCP server task
    async fn tcp_server_task(
        port: u16,
        device_id: String,
        state: NetworkState,
        shutdown_flag: Arc<AtomicBool>
    ) {
        let tcp_port = port + 1;
        let listener = match TcpListener::bind(format!("0.0.0.0:{}", tcp_port)).await {
            Ok(listener) => listener,
            Err(e) => {
                eprintln!("Failed to bind TCP listener: {}", e);
                return;
            }
        };

        loop {
            if shutdown_flag.load(Ordering::Relaxed) {
                break;
            }

            tokio::select! {
                result = listener.accept() => {
                    if let Ok((stream, addr)) = result {
                        let peer_ip = addr.ip();
                        let device_id = device_id.clone();
                        let state = state.clone();
                        let connection_shutdown = Arc::new(AtomicBool::new(false));

                        // Store shutdown signal for this connection
                        state.connection_shutdowns.lock().unwrap().insert(peer_ip, connection_shutdown.clone());

                        tokio::spawn(async move {
                            Self::handle_tcp_connection(stream, peer_ip, device_id, state, connection_shutdown).await;
                        });
                    }
                }
                _ = tokio::time::sleep(Duration::from_millis(100)) => {
                    // Check shutdown flag periodically
                }
            }
        }
    }

    /// Handle TCP connection
    async fn handle_tcp_connection(
        stream: TcpStream,
        peer_ip: IpAddr,
        _device_id: String,
        state: NetworkState,
        connection_shutdown: Arc<AtomicBool>
    ) {
        // Update connection status
        {
            let mut devices = state.devices.lock().unwrap();
            if let Some(device) = devices.get_mut(&peer_ip) {
                device.connection_status = ConnectionStatus::Connected;
                device.intentionally_disconnected = false;
            }
        }

        let (read_half, write_half) = stream.into_split();
        let (tx, mut rx) = mpsc::unbounded_channel::<String>();

        state.connections.lock().unwrap().insert(peer_ip, tx);

        // Handle outgoing messages
        let write_task = {
            let connection_shutdown = connection_shutdown.clone();
            tokio::spawn(async move {
                let mut write_half = write_half;
                loop {
                    if connection_shutdown.load(Ordering::Relaxed) {
                        // Close the write half to signal disconnect to remote
                        let _ = write_half.shutdown().await;
                        break;
                    }

                    tokio::select! {
                        content = rx.recv() => {
                            if let Some(content) = content {
                                if content.len() > 50 * 1024 * 1024 {
                                    continue;
                                }

                                let message = Message::ClipboardData {
                                    device_id: "local".to_string(),
                                    content
                                };
                                if let Ok(json) = serde_json::to_string(&message) {
                                    let json_line = format!("{}\n", json);
                                    if write_half.write_all(json_line.as_bytes()).await.is_err() {
                                        break;
                                    }
                                }
                            } else {
                                break;
                            }
                        }
                        _ = tokio::time::sleep(Duration::from_millis(100)) => {
                            // Check shutdown flag periodically
                        }
                    }
                }
            })
        };

        // Handle incoming messages
        let read_task = {
            let state_clone = state.clone();
            let connection_shutdown = connection_shutdown.clone();
            tokio::spawn(async move {
                let reader = BufReader::new(read_half);
                let mut lines = reader.lines();

                loop {
                    if connection_shutdown.load(Ordering::Relaxed) {
                        break;
                    }

                    tokio::select! {
                        line_result = lines.next_line() => {
                            match line_result {
                                Ok(Some(line)) => {
                                    if let Ok(Message::ClipboardData { content, .. }) = serde_json::from_str(&line) {
                                        if content.len() > 50 * 1024 * 1024 {
                                            continue;
                                        }

                                        let content_hash = ClipboardManager::calculate_hash(&content);

                                        if !state_clone.hash_tracker.contains(content_hash) {
                                            state_clone.hash_tracker.insert(content_hash);

                                            if let Err(_) = ClipboardManager::set_text(&content) {
                                                eprintln!("Failed to set clipboard content");
                                            }

                                            tokio::time::sleep(Duration::from_millis(50)).await;
                                        }
                                    }
                                }
                                Ok(None) => break, // Connection closed
                                Err(_) => break,   // Read error
                            }
                        }
                        _ = tokio::time::sleep(Duration::from_millis(100)) => {
                            // Check shutdown flag periodically
                        }
                    }
                }
            })
        };

        tokio::select! {
            _ = write_task => {},
            _ = read_task => {},
        }

        // Clean up connection
        state.connections.lock().unwrap().remove(&peer_ip);
        state.connection_shutdowns.lock().unwrap().remove(&peer_ip);

        // Update device status based on whether disconnect was intentional
        {
            let mut devices = state.devices.lock().unwrap();
            if let Some(device) = devices.get_mut(&peer_ip) {
                if connection_shutdown.load(Ordering::Relaxed) || device.intentionally_disconnected {
                    device.connection_status = ConnectionStatus::Disconnected;
                } else {
                    device.connection_status = ConnectionStatus::Reconnecting;
                }
            }
        }
    }

    /// Handle remote device disconnect notification
    async fn handle_remote_disconnect(peer_ip: IpAddr, state: &NetworkState) {
        // Signal the connection to shut down
        if let Some(shutdown_signal) = state.connection_shutdowns.lock().unwrap().get(&peer_ip) {
            shutdown_signal.store(true, Ordering::Relaxed);
        }

        // Update device status
        {
            let mut devices = state.devices.lock().unwrap();
            if let Some(device) = devices.get_mut(&peer_ip) {
                device.connection_status = ConnectionStatus::Disconnected;
                device.intentionally_disconnected = true;
            }
        }
    }

    /// Monitor clipboard changes
    async fn clipboard_monitor_task(
        _device_id: String,
        state: NetworkState,
        clipboard_send_tx: mpsc::UnboundedSender<String>,
        shutdown_flag: Arc<AtomicBool>,
    ) {
        let mut last_content_hash = 0u64;
        let mut interval = tokio::time::interval(Duration::from_millis(250));

        loop {
            if shutdown_flag.load(Ordering::Relaxed) {
                break;
            }

            interval.tick().await;

            if let Ok(content) = ClipboardManager::get_text() {
                let content_hash = ClipboardManager::calculate_hash(&content);

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

    /// Connection manager task
    async fn connection_manager_task(
        device_id: String,
        port: u16,
        state: NetworkState,
        mut pair_request_rx: mpsc::UnboundedReceiver<(IpAddr, bool)>,
        mut disconnect_rx: mpsc::UnboundedReceiver<IpAddr>,
        mut clipboard_rx: mpsc::UnboundedReceiver<String>,
        mut gui_clipboard_rx: mpsc::UnboundedReceiver<String>,
        shutdown_flag: Arc<AtomicBool>,
    ) {
        let mut reconnect_interval = tokio::time::interval(Duration::from_secs(3));

        loop {
            if shutdown_flag.load(Ordering::Relaxed) {
                break;
            }

            tokio::select! {
                Some((peer_ip, is_confirm)) = pair_request_rx.recv() => {
                    if is_confirm {
                        Self::send_pair_message(peer_ip, port, Message::PairConfirm { device_id: device_id.clone() }).await;

                        let tcp_port = port + 1;
                        let tcp_addr = SocketAddr::new(peer_ip, tcp_port);
                        if let Ok(stream) = TcpStream::connect(tcp_addr).await {
                            let state_clone = state.clone();
                            let device_id_clone = device_id.clone();
                            let connection_shutdown = Arc::new(AtomicBool::new(false));

                            // Store shutdown signal for this connection
                            state.connection_shutdowns.lock().unwrap().insert(peer_ip, connection_shutdown.clone());

                            tokio::spawn(async move {
                                Self::handle_tcp_connection(stream, peer_ip, device_id_clone, state_clone, connection_shutdown).await;
                            });
                        }
                    } else {
                        Self::send_pair_message(peer_ip, port, Message::PairRequest { device_id: device_id.clone() }).await;
                    }
                }

                Some(peer_ip) = disconnect_rx.recv() => {
                    // Send disconnect message to remote device
                    Self::send_pair_message(peer_ip, port, Message::Disconnect { device_id: device_id.clone() }).await;

                    // Signal the connection to shut down
                    if let Some(shutdown_signal) = state.connection_shutdowns.lock().unwrap().get(&peer_ip) {
                        shutdown_signal.store(true, Ordering::Relaxed);
                    }

                    // Update device status to intentionally disconnected
                    {
                        let mut devices = state.devices.lock().unwrap();
                        if let Some(device) = devices.get_mut(&peer_ip) {
                            device.connection_status = ConnectionStatus::Disconnected;
                            device.intentionally_disconnected = true;
                        }
                    }
                }

                Some(content) = clipboard_rx.recv() => {
                    let connections = state.connections.lock().unwrap();
                    for tx in connections.values() {
                        let _ = tx.send(content.clone());
                    }
                }

                Some(content) = gui_clipboard_rx.recv() => {
                    let connections = state.connections.lock().unwrap();
                    for tx in connections.values() {
                        let _ = tx.send(content.clone());
                    }
                }

                _ = reconnect_interval.tick() => {
                    // Only try to reconnect devices that are in Reconnecting state and not intentionally disconnected
                    let reconnecting_devices: Vec<IpAddr> = {
                        let devices = state.devices.lock().unwrap();
                        devices.iter()
                            .filter(|(_, device)| {
                                device.connection_status == ConnectionStatus::Reconnecting &&
                                !device.intentionally_disconnected
                            })
                            .map(|(ip, _)| *ip)
                            .collect()
                    };

                    for ip in reconnecting_devices {
                        let tcp_port = port + 1;
                        let tcp_addr = SocketAddr::new(ip, tcp_port);
                        if let Ok(stream) = TcpStream::connect(tcp_addr).await {
                            let state_clone = state.clone();
                            let device_id_clone = device_id.clone();
                            let connection_shutdown = Arc::new(AtomicBool::new(false));

                            // Store shutdown signal for this connection
                            state.connection_shutdowns.lock().unwrap().insert(ip, connection_shutdown.clone());

                            tokio::spawn(async move {
                                Self::handle_tcp_connection(stream, ip, device_id_clone, state_clone, connection_shutdown).await;
                            });
                        }
                    }
                }

                _ = tokio::time::sleep(Duration::from_millis(100)) => {
                    // Check shutdown flag periodically
                }
            }
        }
    }

    /// Send pairing message via UDP
    async fn send_pair_message(peer_ip: IpAddr, port: u16, message: Message) {
        if let Ok(socket) = UdpSocket::bind("0.0.0.0:0").await {
            if let Ok(json) = serde_json::to_string(&message) {
                let target = SocketAddr::new(peer_ip, port);
                let _ = socket.send_to(json.as_bytes(), target).await;
            }
        }
    }

    /// Cleanup old devices
    async fn cleanup_task(state: NetworkState, shutdown_flag: Arc<AtomicBool>) {
        let mut interval = tokio::time::interval(Duration::from_secs(5));

        loop {
            if shutdown_flag.load(Ordering::Relaxed) {
                break;
            }

            interval.tick().await;
            let now = Instant::now();

            let mut devices = state.devices.lock().unwrap();
            devices.retain(|_, device| {
                // Don't remove intentionally disconnected devices immediately - keep them for potential reconnection
                if device.intentionally_disconnected && device.connection_status == ConnectionStatus::Disconnected {
                    return true;
                }
                now.duration_since(device.last_seen) < Duration::from_secs(10)
            });
        }
    }
}
