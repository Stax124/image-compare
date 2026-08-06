use eframe::egui::TextureHandle;

/// Minimum width before switching to a stacked mobile layout.
pub const NARROW_BREAKPOINT: f32 = 640.0;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

/// A file picked, dropped, or pasted on the web, delivered as in-memory bytes.
pub struct LoadedFile {
    /// Target side. `None` means auto-route like a single-file drop: first
    /// empty side, otherwise the right side.
    pub side: Option<Side>,
    pub name: String,
    pub bytes: Vec<u8>,
}

/// An image decoded asynchronously by the browser, delivered as raw RGBA8
/// pixels ready to upload as a texture.
pub struct DecodedImage {
    pub side: Side,
    pub name: String,
    pub file_size: usize,
    pub size: [usize; 2],
    pub rgba: Vec<u8>,
}

/// Result of an asynchronous decode. Always delivered (even on failure) so the
/// loading indicator can be cleared.
pub enum DecodeOutcome {
    Ok(DecodedImage),
    Failed,
}

pub struct LoadedImage {
    pub texture: TextureHandle,
    pub name: String,
    pub size: [usize; 2],
    /// Size of the original encoded file in bytes.
    pub file_size: usize,
    /// Raw RGBA8 pixels, kept in memory so edge detection can run without disk access.
    pub rgba: Vec<u8>,
    /// Unique id assigned at load time, used as a cache key for edge detection.
    pub id: u64,
}
