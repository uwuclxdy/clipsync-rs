/// Main application GUI and logic for ClipSync-RS
use crate::network::{NetworkManager, NetworkState, NetworkError};
use crate::types::{ConnectionStatus, Device, NetworkStatus};
use eframe::egui;
use egui::*;
use std::net::IpAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use uuid::Uuid;

/// Communication channels between GUI and networking
pub struct AppChannels {
    pub pair_request_tx: mpsc::UnboundedSender<(IpAddr, bool)>,
    pub disconnect_tx: mpsc::UnboundedSender<IpAddr>,
    pub clipboard_send_tx: mpsc::UnboundedSender<String>,
    pub status_rx: mpsc::UnboundedReceiver<NetworkStatus>,
}

/// Current tab in the interface
#[derive(Debug, Clone, PartialEq)]
enum CurrentTab {
    Devices,
    Settings,
}

/// Enhanced dark theme color palette
struct Colors;

impl Colors {
    const BACKGROUND: Color32 = Color32::from_rgb(15, 20, 30);
    const SURFACE: Color32 = Color32::from_rgb(25, 32, 45);
    const SURFACE_ELEVATED: Color32 = Color32::from_rgb(35, 42, 55);
    const SURFACE_HOVER: Color32 = Color32::from_rgb(45, 52, 65);
    const BORDER: Color32 = Color32::from_rgb(55, 62, 75);
    const BORDER_FOCUS: Color32 = Color32::from_rgb(70, 80, 95);
    const TEXT_PRIMARY: Color32 = Color32::from_rgb(248, 250, 252);
    const TEXT_SECONDARY: Color32 = Color32::from_rgb(160, 170, 185);
    const TEXT_MUTED: Color32 = Color32::from_rgb(115, 125, 140);
    const ACCENT: Color32 = Color32::from_rgb(59, 130, 246);
    const ACCENT_HOVER: Color32 = Color32::from_rgb(37, 99, 235);
    const ACCENT_LIGHT: Color32 = Color32::from_rgb(96, 165, 250);
    const ACCENT_SURFACE: Color32 = Color32::from_rgb(23, 37, 84);
    const SUCCESS: Color32 = Color32::from_rgb(34, 197, 94);
    const SUCCESS_LIGHT: Color32 = Color32::from_rgb(74, 222, 128);
    const SUCCESS_SURFACE: Color32 = Color32::from_rgb(20, 83, 45);
    const WARNING_LIGHT: Color32 = Color32::from_rgb(253, 224, 71);
    const WARNING_SURFACE: Color32 = Color32::from_rgb(113, 63, 18);
    const ERROR: Color32 = Color32::from_rgb(239, 68, 68);
    const ERROR_LIGHT: Color32 = Color32::from_rgb(248, 113, 113);
    const ERROR_SURFACE: Color32 = Color32::from_rgb(127, 29, 29);
    const INFO_LIGHT: Color32 = Color32::from_rgb(168, 85, 247);
    const INFO_SURFACE: Color32 = Color32::from_rgb(67, 20, 107);
    const HEADER_GRADIENT: Color32 = Color32::from_rgb(35, 45, 65);
}

/// Enhanced button styles
struct ButtonStyle;

impl ButtonStyle {
    fn primary() -> (Color32, Color32, Color32) {
        (Colors::ACCENT, Colors::ACCENT_HOVER, Colors::TEXT_PRIMARY)
    }

    fn success() -> (Color32, Color32, Color32) {
        (Colors::SUCCESS, Colors::SUCCESS_LIGHT, Colors::TEXT_PRIMARY)
    }

    fn error() -> (Color32, Color32, Color32) {
        (Colors::ERROR, Colors::ERROR_LIGHT, Colors::TEXT_PRIMARY)
    }

    fn secondary() -> (Color32, Color32, Color32) {
        (Colors::SURFACE_ELEVATED, Colors::SURFACE_HOVER, Colors::TEXT_PRIMARY)
    }
}

/// Main application state
pub struct ClipSyncApp {
    device_id: String,
    port: u16,
    new_port: u16,
    status_message: String,
    error_message: Option<String>,
    current_tab: CurrentTab,
    channels: Option<AppChannels>,
    state: NetworkState,
    networking_active: bool,
    last_scan_time: Instant,
    tab_hover_states: [bool; 2],
    pulse_animation: f32,
    shutdown_flag: Arc<AtomicBool>,
}

impl ClipSyncApp {
    /// Initialize the application with enhanced styling
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        // Configure enhanced dark theme
        let mut style = (*cc.egui_ctx.style()).clone();
        style.visuals = Visuals::dark();
        style.visuals.window_fill = Colors::BACKGROUND;
        style.visuals.panel_fill = Colors::BACKGROUND;
        style.visuals.extreme_bg_color = Colors::SURFACE;
        style.visuals.window_rounding = Rounding::same(16.0);
        style.visuals.menu_rounding = Rounding::same(12.0);
        style.spacing.button_padding = Vec2::new(16.0, 8.0);
        style.spacing.item_spacing = Vec2::new(8.0, 8.0);
        cc.egui_ctx.set_style(style);

        let device_id = Uuid::new_v4().to_string();
        let port = 3847;

        let mut app = Self {
            device_id,
            port,
            new_port: port,
            status_message: "Initializing network...".to_string(),
            error_message: None,
            current_tab: CurrentTab::Devices,
            channels: None,
            state: NetworkState::new(),
            networking_active: false,
            last_scan_time: Instant::now(),
            tab_hover_states: [false; 2],
            pulse_animation: 0.0,
            shutdown_flag: Arc::new(AtomicBool::new(false)),
        };

        app.start_networking_async();
        app
    }

    /// Start networking asynchronously
    fn start_networking_async(&mut self) {
        let device_id = self.device_id.clone();
        let port = self.port;
        let state = self.state.clone();
        let shutdown_flag = self.shutdown_flag.clone();

        let (pair_request_tx, pair_request_rx) = mpsc::unbounded_channel();
        let (disconnect_tx, disconnect_rx) = mpsc::unbounded_channel();
        let (clipboard_send_tx, clipboard_send_rx) = mpsc::unbounded_channel();
        let (status_tx, status_rx) = mpsc::unbounded_channel();

        self.channels = Some(AppChannels {
            pair_request_tx,
            disconnect_tx,
            clipboard_send_tx,
            status_rx,
        });

        let status_tx_clone = status_tx.clone();
        std::thread::spawn(move || {
            let rt = match tokio::runtime::Runtime::new() {
                Ok(rt) => rt,
                Err(e) => {
                    let _ = status_tx.send(NetworkStatus::Error(format!("Failed to create async runtime: {}", e)));
                    return;
                }
            };

            rt.block_on(async {
                match NetworkManager::setup_networking(
                    device_id,
                    port,
                    state,
                    pair_request_rx,
                    disconnect_rx,
                    clipboard_send_rx,
                    status_tx_clone,
                    shutdown_flag
                ).await {
                    Ok(_) => {
                        let _ = status_tx.send(NetworkStatus::StatusUpdate("Network stopped".to_string()));
                    }
                    Err(e) => {
                        let error_msg = match e {
                            NetworkError::PortInUse(port) => format!("Port {} is already in use", port),
                            NetworkError::BindError(msg) => format!("Network bind error: {}", msg),
                        };
                        let _ = status_tx.send(NetworkStatus::Error(error_msg));
                    }
                }
            });
        });

        self.status_message = "Starting network...".to_string();
    }

    /// Restart networking with new port
    fn restart_networking(&mut self) {
        // Signal shutdown to existing networking
        self.shutdown_flag.store(true, Ordering::Relaxed);

        // Clear state
        self.networking_active = false;
        self.status_message = "Restarting network...".to_string();
        self.error_message = None;
        self.channels = None;

        self.state.connections.lock().unwrap().clear();
        self.state.devices.lock().unwrap().clear();

        // Create new shutdown flag
        self.shutdown_flag = Arc::new(AtomicBool::new(false));

        // Give a moment for cleanup
        std::thread::sleep(Duration::from_millis(100));

        self.start_networking_async();
    }

    /// Process network status updates
    fn process_network_status(&mut self) {
        if let Some(channels) = &mut self.channels {
            while let Ok(status) = channels.status_rx.try_recv() {
                match status {
                    NetworkStatus::Started(msg) => {
                        self.networking_active = true;
                        self.status_message = msg;
                        self.error_message = None;
                    }
                    NetworkStatus::Error(msg) => {
                        self.networking_active = false;
                        self.error_message = Some(msg);
                        self.status_message = "Network error".to_string();
                    }
                    NetworkStatus::StatusUpdate(msg) => {
                        self.status_message = msg;
                    }
                }
            }
        }
    }

    /// Enhanced error dialog with better styling
    fn show_error_dialog(&mut self, ctx: &Context) {
        if let Some(error_msg) = &self.error_message.clone() {
            Window::new("Network Error")
                .collapsible(false)
                .resizable(false)
                .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
                .frame(Frame::window(&ctx.style())
                    .fill(Colors::SURFACE_ELEVATED)
                    .stroke(Stroke::new(2.0, Colors::ERROR))
                    .rounding(16.0)
                    .shadow(Shadow::default()))
                .show(ctx, |ui| {
                    ui.add_space(16.0);

                    ui.horizontal(|ui| {
                        ui.colored_label(Colors::ERROR,
                                         RichText::new("ERROR").size(16.0).strong());
                        ui.add_space(12.0);
                        ui.colored_label(Colors::TEXT_PRIMARY,
                                         RichText::new(error_msg).size(14.0));
                    });

                    ui.add_space(20.0);
                    ui.separator();
                    ui.add_space(16.0);

                    ui.horizontal(|ui| {
                        let (bg, _hover, text) = ButtonStyle::primary();
                        if ui.add_sized([120.0, 40.0],
                                        Button::new(RichText::new("Find Free Port").color(text).size(13.0))
                                            .fill(bg)
                                            .rounding(10.0)
                        ).on_hover_cursor(CursorIcon::PointingHand).clicked() {
                            for port in self.new_port + 1..self.new_port + 100 {
                                self.new_port = port;
                                break;
                            }
                            self.port = self.new_port;
                            self.restart_networking();
                        }

                        ui.add_space(12.0);

                        let (bg, _hover, text) = ButtonStyle::secondary();
                        if ui.add_sized([100.0, 40.0],
                                        Button::new(RichText::new("Close").color(text).size(13.0))
                                            .fill(bg)
                                            .rounding(10.0)
                        ).on_hover_cursor(CursorIcon::PointingHand).clicked() {
                            self.error_message = None;
                        }
                    });

                    ui.add_space(16.0);
                });
        }
    }

    /// Enhanced header with professional gradient
    fn draw_header(&mut self, ui: &mut Ui) {
        // Professional gradient background
        let header_rect = Rect::from_min_size(ui.min_rect().min, Vec2::new(ui.available_width(), 80.0));
        ui.painter().rect_filled(header_rect, 0.0, Colors::HEADER_GRADIENT);

        ui.add_space(24.0);

        ui.horizontal(|ui| {
            ui.add_space(24.0);

            // App branding
            ui.colored_label(Colors::TEXT_PRIMARY,
                             RichText::new("ClipSync-RS").size(24.0).strong());

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(24.0);

                // Connection counter
                let connected_count = self.state.devices.lock().unwrap()
                    .values()
                    .filter(|d| d.connection_status == ConnectionStatus::Connected)
                    .count();

                if connected_count > 0 {
                    Frame::none()
                        .fill(Colors::SUCCESS_SURFACE)
                        .rounding(24.0)
                        .inner_margin(Margin::symmetric(16.0, 8.0))
                        .stroke(Stroke::new(1.5, Colors::SUCCESS))
                        .show(ui, |ui| {
                            ui.colored_label(Colors::SUCCESS_LIGHT,
                                             RichText::new(format!("{} connected", connected_count)).size(13.0).strong());
                        });
                    ui.add_space(20.0);
                }

                // Status indicator
                let (status_color, status_bg) = if self.networking_active {
                    (Colors::SUCCESS_LIGHT, Colors::SUCCESS_SURFACE)
                } else if self.error_message.is_some() {
                    (Colors::ERROR_LIGHT, Colors::ERROR_SURFACE)
                } else {
                    (Colors::WARNING_LIGHT, Colors::WARNING_SURFACE)
                };

                Frame::none()
                    .fill(status_bg)
                    .rounding(24.0)
                    .inner_margin(Margin::symmetric(16.0, 8.0))
                    .stroke(Stroke::new(1.5, status_color))
                    .show(ui, |ui| {
                        ui.colored_label(status_color,
                                         RichText::new(&self.status_message).size(13.0).strong());
                    });
            });
        });

        ui.add_space(20.0);

        // Separator
        let separator_rect = Rect::from_min_size(
            ui.min_rect().min + Vec2::new(24.0, 0.0),
            Vec2::new(ui.available_width() - 48.0, 2.0)
        );
        ui.painter().rect_filled(separator_rect, 1.0, Colors::BORDER_FOCUS);
    }

    /// Tab navigation with hover effects
    fn draw_tabs(&mut self, ui: &mut Ui) {
        ui.add_space(24.0);

        ui.horizontal(|ui| {
            ui.add_space(24.0);

            let tab_data = [
                ("Devices", CurrentTab::Devices, 0),
                ("Settings", CurrentTab::Settings, 1),
            ];

            for (text, tab, index) in tab_data.iter() {
                let is_active = self.current_tab == *tab;
                let is_hovered = self.tab_hover_states[*index];

                let (text_color, bg_color, border_color) = if is_active {
                    (Colors::ACCENT_LIGHT, Colors::ACCENT_SURFACE, Colors::ACCENT)
                } else if is_hovered {
                    (Colors::TEXT_PRIMARY, Colors::SURFACE_HOVER, Colors::BORDER_FOCUS)
                } else {
                    (Colors::TEXT_SECONDARY, Color32::TRANSPARENT, Color32::TRANSPARENT)
                };

                let button = ui.add_sized([140.0, 44.0],
                                          Button::new(RichText::new(*text).color(text_color).size(14.0).strong())
                                              .fill(bg_color)
                                              .stroke(Stroke::new(1.5, border_color))
                                              .rounding(12.0)
                ).on_hover_cursor(CursorIcon::PointingHand);

                self.tab_hover_states[*index] = button.hovered();

                if button.clicked() {
                    self.current_tab = tab.clone();
                }

                ui.add_space(12.0);
            }
        });

        ui.add_space(24.0);
    }

    /// Devices tab with scrollable device list
    fn draw_devices_tab(&mut self, ui: &mut Ui) {
        ui.add_space(16.0);

        // Section header
        ui.horizontal(|ui| {
            ui.add_space(24.0);
            ui.colored_label(Colors::TEXT_PRIMARY,
                             RichText::new("Network Devices").size(20.0).strong());
        });

        ui.add_space(20.0);

        let devices = self.state.devices.lock().unwrap().clone();

        if devices.is_empty() {
            // Empty state
            Frame::none()
                .fill(Colors::SURFACE)
                .rounding(20.0)
                .inner_margin(48.0)
                .outer_margin(Margin::symmetric(24.0, 0.0))
                .stroke(Stroke::new(1.5, Colors::BORDER))
                .shadow(Shadow::default())
                .show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.add_space(8.0);

                        if self.networking_active {
                            // Scanning indicator with pulse effect
                            let pulse_alpha = (self.pulse_animation.sin() * 0.4 + 0.6).clamp(0.3, 1.0);
                            ui.colored_label(
                                Color32::from_rgba_premultiplied(
                                    Colors::ACCENT.r(),
                                    Colors::ACCENT.g(),
                                    Colors::ACCENT.b(),
                                    (255.0 * pulse_alpha) as u8
                                ),
                                RichText::new("SCANNING").size(16.0).strong()
                            );

                            ui.add_space(16.0);
                            ui.colored_label(Colors::TEXT_PRIMARY,
                                             RichText::new("Scanning for devices...").size(18.0).strong());
                            ui.add_space(8.0);
                            ui.colored_label(Colors::TEXT_MUTED,
                                             RichText::new("Make sure other ClipSync-RS instances are running on your network").size(14.0));

                            ui.add_space(20.0);

                            // Progress indicator
                            let elapsed = self.last_scan_time.elapsed().as_secs();
                            let dots = match elapsed % 4 {
                                0 => "",
                                1 => ".",
                                2 => "..",
                                _ => "...",
                            };

                            Frame::none()
                                .fill(Colors::ACCENT_SURFACE)
                                .rounding(16.0)
                                .inner_margin(Margin::symmetric(12.0, 6.0))
                                .stroke(Stroke::new(1.0, Colors::ACCENT))
                                .show(ui, |ui| {
                                    ui.colored_label(Colors::ACCENT_LIGHT,
                                                     RichText::new(format!("Scanning{}", dots)).size(13.0).strong());
                                });
                        } else {
                            // Network not active
                            ui.colored_label(Colors::ERROR,
                                             RichText::new("NETWORK INACTIVE").size(16.0).strong());
                            ui.add_space(16.0);
                            ui.colored_label(Colors::TEXT_PRIMARY,
                                             RichText::new("Network is not running").size(18.0).strong());
                            ui.add_space(8.0);
                            ui.colored_label(Colors::TEXT_MUTED,
                                             RichText::new("Check the error message or try restarting").size(14.0));
                        }

                        ui.add_space(8.0);
                    });
                });
        } else {
            // Scrollable device list
            let available_height = ui.available_height() - 40.0;

            ScrollArea::vertical()
                .max_height(available_height)
                .auto_shrink([false; 2])
                .show(ui, |ui| {
                    ui.add_space(12.0);
                    for (ip, device) in devices.iter() {
                        self.draw_device_card(ui, *ip, device);
                        ui.add_space(12.0);
                    }
                    ui.add_space(12.0);
                });
        }
    }

    /// Device card with clean design
    fn draw_device_card(&mut self, ui: &mut Ui, ip: IpAddr, device: &Device) {
        let (status_color, status_bg, status_text, button_text, button_style, button_enabled) =
            match device.connection_status {
                ConnectionStatus::Disconnected => {
                    if device.intentionally_disconnected {
                        (
                            Colors::TEXT_MUTED, Colors::SURFACE_ELEVATED, "DISCONNECTED",
                            "Reconnect", ButtonStyle::primary(), true
                        )
                    } else {
                        (
                            Colors::TEXT_MUTED, Colors::SURFACE_ELEVATED, "DISCONNECTED",
                            "Connect", ButtonStyle::primary(), true
                        )
                    }
                },
                ConnectionStatus::Connecting => (
                    Colors::WARNING_LIGHT, Colors::WARNING_SURFACE, "PENDING",
                    "Accept", ButtonStyle::success(), true
                ),
                ConnectionStatus::Connected => (
                    Colors::SUCCESS_LIGHT, Colors::SUCCESS_SURFACE, "CONNECTED",
                    "Disconnect", ButtonStyle::error(), true
                ),
                ConnectionStatus::Reconnecting => (
                    Colors::INFO_LIGHT, Colors::INFO_SURFACE, "CONNECTING",
                    "", ButtonStyle::secondary(), false
                ),
            };

        Frame::none()
            .fill(Colors::SURFACE)
            .rounding(16.0)
            .inner_margin(24.0)
            .outer_margin(Margin::symmetric(24.0, 0.0))
            .stroke(Stroke::new(1.5, Colors::BORDER))
            .shadow(Shadow::default())
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    // Status indicator
                    Frame::none()
                        .fill(status_bg)
                        .rounding(28.0)
                        .inner_margin(Margin::symmetric(12.0, 8.0))
                        .stroke(Stroke::new(1.5, status_color))
                        .show(ui, |ui| {
                            ui.colored_label(status_color,
                                             RichText::new(status_text).size(11.0).strong());
                        });

                    ui.add_space(20.0);

                    // Device info
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            ui.colored_label(Colors::TEXT_MUTED,
                                             RichText::new("IP:").size(12.0));
                            ui.add_space(6.0);
                            ui.colored_label(Colors::TEXT_PRIMARY,
                                             RichText::new(format!("{}", ip)).size(16.0).strong());
                        });
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            ui.colored_label(Colors::TEXT_MUTED,
                                             RichText::new("ID:").size(12.0));
                            ui.add_space(6.0);
                            ui.colored_label(Colors::TEXT_SECONDARY,
                                             RichText::new(&device.device_id[..8]).size(12.0).monospace());
                        });
                    });

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        // Action button
                        if button_enabled && !button_text.is_empty() {
                            let (bg_color, hover_color, text_color) = button_style;

                            let button = ui.add_sized([110.0, 40.0],
                                                      Button::new(RichText::new(button_text).color(text_color).size(13.0).strong())
                                                          .fill(bg_color)
                                                          .rounding(10.0)
                                                          .stroke(Stroke::new(1.0, bg_color))
                            ).on_hover_cursor(CursorIcon::PointingHand);

                            if button.hovered() {
                                ui.painter().rect_filled(button.rect, 10.0, hover_color);
                            }

                            if button.clicked() {
                                if let Some(channels) = &self.channels {
                                    match device.connection_status {
                                        ConnectionStatus::Disconnected => {
                                            // Clear intentionally disconnected flag when reconnecting
                                            if device.intentionally_disconnected {
                                                let mut devices = self.state.devices.lock().unwrap();
                                                if let Some(dev) = devices.get_mut(&ip) {
                                                    dev.intentionally_disconnected = false;
                                                }
                                            }
                                            let _ = channels.pair_request_tx.send((ip, false));
                                        }
                                        ConnectionStatus::Connecting => {
                                            let _ = channels.pair_request_tx.send((ip, true));
                                        }
                                        ConnectionStatus::Connected => {
                                            let _ = channels.disconnect_tx.send(ip);
                                        }
                                        _ => {}
                                    }
                                }
                            }
                        }
                    });
                });
            });
    }

    /// Settings tab with configuration options
    fn draw_settings_tab(&mut self, ui: &mut Ui) {
        ui.add_space(16.0);

        ScrollArea::vertical().show(ui, |ui| {
            // Network Settings Card
            Frame::none()
                .fill(Colors::SURFACE)
                .rounding(20.0)
                .inner_margin(28.0)
                .outer_margin(Margin::symmetric(24.0, 0.0))
                .stroke(Stroke::new(1.5, Colors::BORDER))
                .shadow(Shadow::default())
                .show(ui, |ui| {
                    ui.colored_label(Colors::TEXT_PRIMARY,
                                     RichText::new("Network Settings").size(20.0).strong());

                    ui.add_space(24.0);

                    // Port setting
                    ui.horizontal(|ui| {
                        ui.colored_label(Colors::TEXT_PRIMARY,
                                         RichText::new("Port:").size(15.0).strong());
                        ui.add_space(16.0);

                        if ui.add_sized([140.0, 32.0],
                                        DragValue::new(&mut self.new_port)
                                            .range(1024..=65535)
                                            .speed(0.1)
                        ).changed() {
                            if self.new_port != self.port {
                                self.port = self.new_port;
                                self.restart_networking();
                            }
                        }

                        ui.add_space(20.0);
                        ui.colored_label(Colors::TEXT_MUTED,
                                         RichText::new("(Changes applied immediately)").size(12.0));
                    });

                    ui.add_space(20.0);

                    // Device ID display
                    ui.horizontal(|ui| {
                        ui.colored_label(Colors::TEXT_PRIMARY,
                                         RichText::new("Device ID:").size(15.0).strong());
                        ui.add_space(16.0);

                        Frame::none()
                            .fill(Colors::SURFACE_ELEVATED)
                            .rounding(8.0)
                            .inner_margin(Margin::symmetric(10.0, 6.0))
                            .stroke(Stroke::new(1.0, Colors::BORDER))
                            .show(ui, |ui| {
                                ui.colored_label(Colors::TEXT_SECONDARY,
                                                 RichText::new(&self.device_id[..8]).monospace().size(13.0));
                            });

                        ui.add_space(12.0);
                        ui.colored_label(Colors::TEXT_MUTED,
                                         RichText::new("(Auto-generated UUID)").size(12.0));
                    });
                });

            ui.add_space(16.0);

            // Performance Settings Card
            Frame::none()
                .fill(Colors::SURFACE)
                .rounding(20.0)
                .inner_margin(28.0)
                .outer_margin(Margin::symmetric(24.0, 0.0))
                .stroke(Stroke::new(1.5, Colors::BORDER))
                .shadow(Shadow::default())
                .show(ui, |ui| {
                    ui.colored_label(Colors::TEXT_PRIMARY,
                                     RichText::new("Performance").size(20.0).strong());

                    ui.add_space(24.0);

                    // Performance metrics
                    let metrics = [
                        ("Clipboard polling", "250ms", "Optimized for responsiveness"),
                        ("Max clipboard size", "50MB", "Prevents network congestion"),
                        ("Discovery interval", "5s", "Balance of speed and efficiency"),
                    ];

                    for (label, value, description) in metrics.iter() {
                        ui.horizontal(|ui| {
                            ui.colored_label(Colors::TEXT_PRIMARY,
                                             RichText::new(format!("{}:", label)).size(15.0));
                            ui.add_space(16.0);

                            Frame::none()
                                .fill(Colors::ACCENT_SURFACE)
                                .rounding(8.0)
                                .inner_margin(Margin::symmetric(10.0, 6.0))
                                .stroke(Stroke::new(1.0, Colors::ACCENT))
                                .show(ui, |ui| {
                                    ui.colored_label(Colors::ACCENT_LIGHT,
                                                     RichText::new(*value).monospace().size(13.0).strong());
                                });

                            ui.add_space(20.0);
                            ui.colored_label(Colors::TEXT_MUTED,
                                             RichText::new(*description).size(12.0));
                        });
                        ui.add_space(16.0);
                    }
                });
        });
    }
}

impl eframe::App for ClipSyncApp {
    /// Main GUI update loop
    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        // Process network status updates
        self.process_network_status();

        // Update pulse animation for scanning indicator
        self.pulse_animation += 0.04;
        if self.pulse_animation > std::f32::consts::PI * 2.0 {
            self.pulse_animation = 0.0;
        }

        // Show error dialog if needed
        self.show_error_dialog(ctx);

        // Main panel
        CentralPanel::default()
            .frame(Frame::central_panel(&ctx.style())
                .fill(Colors::BACKGROUND)
                .inner_margin(0.0))
            .show(ctx, |ui| {
                self.draw_header(ui);
                self.draw_tabs(ui);

                match self.current_tab {
                    CurrentTab::Devices => self.draw_devices_tab(ui),
                    CurrentTab::Settings => self.draw_settings_tab(ui),
                }
            });

        // Request regular repaints for animations
        ctx.request_repaint_after(Duration::from_millis(60));
    }
}
