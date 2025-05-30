# Clipboard Sync

A fast, secure, cross-platform clipboard synchronization tool for local networks. Share clipboard content seamlessly between multiple devices with a simple peer-to-peer architecture.

## Features

| Feature | Description |
|---------|-------------|
| **Cross-Platform** | Works on Windows, Linux, and macOS |
| **Peer-to-Peer** | No central server required - direct device communication |
| **Auto Discovery** | Automatically finds other instances on the local network |
| **Manual Pairing** | Secure connection establishment with mutual confirmation |
| **Multi-Device** | Connect to multiple devices simultaneously |
| **Real-Time Sync** | Instant clipboard synchronization across all connected devices |
| **Auto Reconnect** | Automatically reconnects when devices come back online |
| **Large Content** | Supports clipboard content up to 50MB |
| **Simple GUI** | Clean, intuitive interface with status indicators |
| **Network Aware** | Broadcasts across all network interfaces |

## Use Cases

- **VM ↔ Host**: Sync clipboard between virtual machines and host systems
- **Development**: Share code snippets between development machines
- **Multi-Monitor**: Work seamlessly across multiple computers
- **Team Collaboration**: Share clipboard content in local office environments

## Installation

### From Source

1. Install Rust: https://rustup.rs/
2. Clone and build:
```bash
git clone <repository-url>
cd clipboard-sync
cargo run --release
```

### Dependencies

The app will automatically install required dependencies:
- `egui` - GUI framework
- `arboard` - Cross-platform clipboard access
- `tokio` - Async networking
- `serde` - Message serialization
- `if-addrs` - Network interface detection

## Usage

1. **Start the application** on each device you want to sync
2. **Configure port** (default: 3847) if needed in the Configuration panel
3. **Wait for discovery** - devices will appear in the "Devices" list
4. **Connect devices**:
   - Click "Connect" on Device A
   - Click "Accept" on Device B when it shows "Wants to connect"
5. **Start syncing** - clipboard content will automatically sync between connected devices

### Status Indicators

- ⚪ **Disconnected**: Ready to connect
- 🟡 **Connecting**: Remote device wants to connect (click "Accept")
- 🟢 **Connected**: Actively syncing clipboard
- 🟠 **Reconnecting**: Attempting to restore lost connection

## Technical Details

### Architecture
- **Discovery**: UDP broadcast messages for device detection
- **Pairing**: Manual confirmation protocol for secure connections  
- **Data Transfer**: TCP connections for reliable clipboard synchronization
- **Multi-Interface**: Broadcasts across all active network interfaces

### Network Ports
- **UDP Port**: 3847 (configurable) - Device discovery
- **TCP Port**: 3848 (port + 1) - Clipboard data transfer

### Security
- Local network only - no internet communication
- Manual pairing confirmation required
- No data persistence - clipboard content only exists in memory
- Unique device IDs prevent accidental connections

## Configuration

- **Port**: Change the UDP discovery port (1024-65535)
- **Device ID**: Automatically generated unique identifier
- **Multi-Interface**: Automatically uses all available network interfaces

## Troubleshooting

### No Devices Found
- Ensure both devices are on the same network
- Check firewall settings for UDP port 3847
- Verify network interfaces are active

### Connection Failed
- Check TCP port 3848 is not blocked
- Ensure devices can reach each other on the network
- Try restarting both applications

### Clipboard Not Syncing
- Verify connection status shows "Connected" (🟢)
- Check if clipboard content exceeds 50MB limit
- Restart the application if sync stops working

## Building

### Release Build
```bash
cargo build --release
```

### Debug Build (with networking logs)
```bash
RUST_LOG=debug cargo run
```

## System Requirements

- **OS**: Windows 10+, Linux (X11/Wayland), macOS 10.12+
- **RAM**: 50MB minimum
- **Network**: Local area network connection
- **Permissions**: Clipboard access, network socket binding

## License

MIT License - see LICENSE file for details.

## Contributing

Contributions welcome! Please feel free to submit issues and pull requests.

---

**Note**: This tool is designed for local network use only. Always ensure you trust the devices you're connecting to.