use eframe::egui::TextureHandle;

/// Minimum width before switching to a stacked mobile layout.
pub const NARROW_BREAKPOINT: f32 = 640.0;

/// Which half of the comparison a file or texture belongs to.
///
/// Discriminant values are stable so `Side` can index `[T; 2]` arrays.
#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(usize)]
pub enum Side {
    Left = 0,
    Right = 1,
}

impl Side {
    pub const ALL: [Side; 2] = [Side::Left, Side::Right];

    #[inline]
    pub fn index(self) -> usize {
        self as usize
    }

    pub fn label(self) -> &'static str {
        match self {
            Side::Left => "Left",
            Side::Right => "Right",
        }
    }

    /// Short label used in the narrow/mobile header.
    pub fn short_label(self) -> &'static str {
        match self {
            Side::Left => "L",
            Side::Right => "R",
        }
    }
}

/// A file picked, dropped, or pasted on the web, delivered as in-memory bytes.
pub struct LoadedFile {
    /// Target side. `None` means auto-route: first empty side, else right.
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
    /// Raw RGBA8 pixels, kept so edge detection can run without re-decoding.
    pub rgba: Vec<u8>,
    /// Unique id assigned at load time; cheap cache key for edge detection.
    pub id: u64,
}
