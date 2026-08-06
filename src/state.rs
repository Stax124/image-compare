use eframe::egui::{self, TextureHandle, Vec2};
use std::sync::mpsc;

use crate::types::{DecodeOutcome, LoadedFile, Side};

/// Returns a process-unique, monotonically increasing id for each loaded image.
/// Used as a cheap cache key for edge detection instead of hashing every pixel.
pub fn next_image_id() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    COUNTER.fetch_add(1, Ordering::Relaxed)
}

/// Pan/zoom/separator state for the comparison view.
pub struct ViewState {
    /// Separator position as a fraction of the canvas width (0..1).
    pub separator: f32,
    pub zoom: f32,
    pub pan_offset: Vec2,
    pub dragging_separator: bool,
    pub panning: bool,
    pub last_pan_pos: Option<egui::Pos2>,
}

impl Default for ViewState {
    fn default() -> Self {
        Self {
            separator: 0.5,
            zoom: 1.0,
            pan_offset: Vec2::ZERO,
            dragging_separator: false,
            panning: false,
            last_pan_pos: None,
        }
    }
}

impl ViewState {
    /// Reset zoom and pan, leaving the separator untouched.
    pub fn reset_zoom_pan(&mut self) {
        self.zoom = 1.0;
        self.pan_offset = Vec2::ZERO;
    }
}

/// Cached edge texture for one side, keyed by the source image's `id`.
#[derive(Default)]
pub struct EdgeCache {
    pub texture: Option<TextureHandle>,
    pub key: Option<u64>,
}

impl EdgeCache {
    pub fn clear(&mut self) {
        self.texture = None;
        self.key = None;
    }

    /// Whether the cache is missing or was built for a different image.
    pub fn needs_recompute(&self, image_id: u64) -> bool {
        self.texture.is_none() || self.key != Some(image_id)
    }
}

/// Edge display polarity: magnitude as light-on-dark or dark-on-light.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum EdgePolarity {
    /// White edges on black background (Sobel magnitude as-is).
    #[default]
    Negative,
    /// Black edges on white background (inverted magnitude).
    Positive,
}

/// Edge-detection toggle plus per-side cached edge textures.
#[derive(Default)]
pub struct EdgeState {
    pub enabled: bool,
    pub polarity: EdgePolarity,
    pub caches: [EdgeCache; 2],
}

impl EdgeState {
    pub fn cache(&self, side: Side) -> &EdgeCache {
        &self.caches[side.index()]
    }

    pub fn cache_mut(&mut self, side: Side) -> &mut EdgeCache {
        &mut self.caches[side.index()]
    }

    pub fn clear_side(&mut self, side: Side) {
        self.caches[side.index()].clear();
    }
}

/// Channels and counters coordinating asynchronous file picking and decoding.
pub struct AsyncIo {
    pub file_tx: mpsc::Sender<LoadedFile>,
    pub file_rx: mpsc::Receiver<LoadedFile>,
    pub decoded_tx: mpsc::Sender<DecodeOutcome>,
    pub decoded_rx: mpsc::Receiver<DecodeOutcome>,
    /// Number of images currently being decoded. While > 0 a loading screen is shown.
    pub pending_loads: usize,
}

impl Default for AsyncIo {
    fn default() -> Self {
        let (file_tx, file_rx) = mpsc::channel();
        let (decoded_tx, decoded_rx) = mpsc::channel();
        Self {
            file_tx,
            file_rx,
            decoded_tx,
            decoded_rx,
            pending_loads: 0,
        }
    }
}

/// Tracks an in-progress drag-and-drop gesture so files trickling in across
/// frames can be routed to the correct side.
#[derive(Default)]
pub struct DropState {
    /// Whether a drag gesture was hovering files over the window last frame.
    pub was_hovering: bool,
    /// Number of files in the current/most recent drag gesture.
    pub expected: usize,
    /// How many files of the current drop gesture have been routed so far.
    pub index: usize,
}
