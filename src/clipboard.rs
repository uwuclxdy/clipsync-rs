// clipboard.rs
/// Cross-platform clipboard operations with fallbacks
use std::error::Error;

pub struct ClipboardManager;

impl ClipboardManager {
    /// Set clipboard with Linux fallback
    pub fn set_text(content: &str) -> Result<(), Box<dyn Error>> {
        if let Ok(mut clipboard) = arboard::Clipboard::new() {
            clipboard.set_text(content)?;
            return Ok(());
        }

        // Linux fallback - try creating fresh clipboard instance
        if cfg!(target_os = "linux") {
            if let Ok(mut fresh_clipboard) = arboard::Clipboard::new() {
                fresh_clipboard.set_text(content)?;
                return Ok(());
            }
        }

        Err("Failed to access clipboard".into())
    }

    /// Get clipboard with Linux fallback
    pub fn get_text() -> Result<String, Box<dyn Error>> {
        if let Ok(mut clipboard) = arboard::Clipboard::new() {
            if let Ok(content) = clipboard.get_text() {
                return Ok(content);
            }
        }

        // Linux fallback
        if cfg!(target_os = "linux") {
            if let Ok(mut fresh_clipboard) = arboard::Clipboard::new() {
                return fresh_clipboard.get_text().map_err(|e| e.into());
            }
        }

        Err("Failed to access clipboard".into())
    }

    /// Calculate deterministic hash for content comparison
    pub fn calculate_hash(content: &str) -> u64 {
        const FNV_OFFSET_BASIS: u64 = 14695981039346656037;
        const FNV_PRIME: u64 = 1099511628211;

        let mut hash = FNV_OFFSET_BASIS;
        for byte in content.bytes() {
            hash ^= byte as u64;
            hash = hash.wrapping_mul(FNV_PRIME);
        }
        hash
    }
}
