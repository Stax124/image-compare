use eframe::egui;
use egui::{TextureHandle, TextureOptions, Vec2};
use std::sync::mpsc;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

/// A file picked or dropped on the web, delivered as in-memory bytes.
pub struct LoadedFile {
    pub side: Side,
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

/// Returns a process-unique, monotonically increasing id for each loaded image.
/// Used as a cheap cache key for edge detection instead of hashing every pixel.
fn next_image_id() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    COUNTER.fetch_add(1, Ordering::Relaxed)
}

/// Current high-resolution timestamp in milliseconds, used for timing logs.
/// Falls back to 0.0 if the Performance API is unavailable.
fn now_ms() -> f64 {
    web_sys::window()
        .and_then(|w| w.performance())
        .map(|p| p.now())
        .unwrap_or(0.0)
}

/// Format a byte count into a human-readable string (e.g. "1.2 MB").
pub fn format_file_size(bytes: usize) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{} {}", bytes, UNITS[unit])
    } else {
        format!("{:.1} {}", size, UNITS[unit])
    }
}

/// Truncate a file name to at most `max` characters, appending an ellipsis when
/// it was shortened. Keeps compact image info from overflowing on mobile.
fn truncate_name(name: &str, max: usize) -> String {
    if name.chars().count() <= max {
        name.to_string()
    } else {
        let keep: String = name.chars().take(max.saturating_sub(1)).collect();
        format!("{keep}…")
    }
}

pub struct App {
    pub left: Option<LoadedImage>,
    pub right: Option<LoadedImage>,
    pub separator: f32,
    pub dragging_separator: bool,
    pub zoom: f32,
    pub pan_offset: Vec2,
    pub panning: bool,
    pub last_pan_pos: Option<egui::Pos2>,
    pub edge_detect: bool,
    pub left_edge: Option<TextureHandle>,
    pub right_edge: Option<TextureHandle>,
    /// Hash of the input the cached `left_edge` was computed from.
    pub left_edge_key: Option<u64>,
    /// Hash of the input the cached `right_edge` was computed from.
    pub right_edge_key: Option<u64>,
    pub file_tx: mpsc::Sender<LoadedFile>,
    pub file_rx: mpsc::Receiver<LoadedFile>,
    pub decoded_tx: mpsc::Sender<DecodeOutcome>,
    pub decoded_rx: mpsc::Receiver<DecodeOutcome>,
    /// Whether a drag gesture was hovering files over the window last frame.
    pub was_hovering: bool,
    /// Number of files in the current/most recent drag gesture (known during
    /// dragover, before the drop's bytes arrive).
    pub drop_expected: usize,
    /// How many files of the current drop gesture have been routed so far.
    pub drop_index: usize,
    /// Number of images currently being decoded asynchronously. While > 0 a
    /// loading screen is shown instead of the drop zones.
    pub pending_loads: usize,
}

impl Default for App {
    fn default() -> Self {
        let (file_tx, file_rx) = mpsc::channel();
        let (decoded_tx, decoded_rx) = mpsc::channel();
        Self {
            left: None,
            right: None,
            separator: 0.5,
            dragging_separator: false,
            zoom: 1.0,
            pan_offset: Vec2::ZERO,
            panning: false,
            last_pan_pos: None,
            edge_detect: false,
            left_edge: None,
            right_edge: None,
            left_edge_key: None,
            right_edge_key: None,
            file_tx,
            file_rx,
            decoded_tx,
            decoded_rx,
            was_hovering: false,
            drop_expected: 0,
            drop_index: 0,
            pending_loads: 0,
        }
    }
}

impl App {
    /// Build a `LoadedImage` (texture + cached pixels) from already-decoded
    /// RGBA8 pixel data.
    fn build_loaded_image(
        &self,
        ctx: &egui::Context,
        name: &str,
        file_size: usize,
        size: [usize; 2],
        raw: Vec<u8>,
    ) -> LoadedImage {
        let t_start = now_ms();
        let color_image = egui::ColorImage::from_rgba_unmultiplied(size, &raw);
        let t_color = now_ms();
        let texture = ctx.load_texture(name, color_image, TextureOptions::LINEAR);
        let t_texture = now_ms();
        let id = next_image_id();

        log::info!(
            "[timing] build_loaded_image '{}' ({}x{}): ColorImage {:.1}ms, upload texture {:.1}ms, total {:.1}ms",
            name,
            size[0],
            size[1],
            t_color - t_start,
            t_texture - t_color,
            t_texture - t_start,
        );

        LoadedImage {
            texture,
            name: name.to_string(),
            size,
            file_size,
            rgba: raw,
            id,
        }
    }

    /// Assign a loaded image to one side, clearing that side's cached edges.
    fn set_side(&mut self, side: Side, loaded: LoadedImage) {
        match side {
            Side::Left => {
                self.left = Some(loaded);
                self.left_edge = None;
                self.left_edge_key = None;
            }
            Side::Right => {
                self.right = Some(loaded);
                self.right_edge = None;
                self.right_edge_key = None;
            }
        }
    }

    /// Decode `bytes` and assign the result to `side`. Decoding is delegated to
    /// the browser's native image decoder and arrives asynchronously via
    /// `decoded_rx`.
    fn load_and_set(&mut self, ctx: &egui::Context, side: Side, name: &str, bytes: &[u8]) {
        self.pending_loads += 1;
        decode_via_browser(
            ctx.clone(),
            self.decoded_tx.clone(),
            side,
            name.to_string(),
            bytes.to_vec(),
        );
    }

    pub fn handle_dropped_files(&mut self, ctx: &egui::Context) {
        // On the web each dropped file is read by its own async future and
        // pushed into `dropped_files` in a separate frame, so a multi-file drop
        // never arrives all at once. Track the drag-hover state to learn how
        // many files belong to the current drop and route them deterministically
        // as their bytes trickle in.
        let hovering_count = ctx.input(|i| i.raw.hovered_files.len());
        if hovering_count > 0 {
            if !self.was_hovering {
                // A new drag gesture started: reset the per-drop routing index.
                self.drop_index = 0;
            }
            // Number of files being dragged, available during dragover.
            self.drop_expected = hovering_count;
        }
        self.was_hovering = hovering_count > 0;

        let dropped_files: Vec<(String, Vec<u8>)> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.bytes.as_ref().map(|b| (f.name.clone(), b.to_vec())))
                .collect()
        });

        for (name, bytes) in dropped_files {
            let side = if self.drop_expected >= 2 {
                // Multi-file drop: first file -> left, second -> right, ignore
                // any extras.
                match self.drop_index {
                    0 => Side::Left,
                    1 => Side::Right,
                    _ => {
                        self.drop_index += 1;
                        continue;
                    }
                }
            } else {
                // Single-file drop: fill the first empty side, else the right.
                if self.left.is_none() {
                    Side::Left
                } else {
                    Side::Right
                }
            };
            self.drop_index += 1;
            self.load_and_set(ctx, side, &name, &bytes);
        }
    }

    /// Open the native browser file picker directly (no confirmation dialog) and
    /// deliver the picked file's bytes over a channel.
    pub fn request_open(&self, ctx: &egui::Context, side: Side) {
        use wasm_bindgen::JsCast;
        use wasm_bindgen::closure::Closure;

        let tx = self.file_tx.clone();
        let ctx = ctx.clone();
        let window = match web_sys::window() {
            Some(w) => w,
            None => return,
        };
        let document = match window.document() {
            Some(d) => d,
            None => return,
        };
        let input: web_sys::HtmlInputElement = match document
            .create_element("input")
            .ok()
            .and_then(|el| el.dyn_into().ok())
        {
            Some(i) => i,
            None => return,
        };
        input.set_type("file");
        input.set_accept(
            "image/png,image/jpeg,image/webp,image/bmp,image/tiff,image/avif,.png,.jpg,.jpeg,.webp,.bmp,.tiff,.avif",
        );

        let input_clone = input.clone();
        let closure = Closure::<dyn FnMut()>::new(move || {
            let Some(files) = input_clone.files() else {
                return;
            };
            let Some(file) = files.get(0) else {
                return;
            };
            let name = file.name();
            let tx = tx.clone();
            let ctx = ctx.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let Ok(buffer) = wasm_bindgen_futures::JsFuture::from(file.array_buffer()).await
                else {
                    return;
                };
                let bytes = js_sys::Uint8Array::new(&buffer).to_vec();
                if tx.send(LoadedFile { side, name, bytes }).is_ok() {
                    // Wake the UI so the picked file is polled and decoded even
                    // when no other input is driving frames (e.g. on mobile).
                    ctx.request_repaint();
                }
            });
        });
        input.set_onchange(Some(closure.as_ref().unchecked_ref()));
        // The closure must outlive this function so the change event can fire.
        closure.forget();

        input.click();
    }

    /// Drain any files picked through the async dialog and apply them.
    pub fn poll_picked_files(&mut self, ctx: &egui::Context) {
        let pending: Vec<LoadedFile> = self.file_rx.try_iter().collect();
        for file in pending {
            self.load_and_set(ctx, file.side, &file.name, &file.bytes);
        }
    }

    /// Drain any images decoded asynchronously by the browser (e.g. AVIF) and
    /// apply them to their target side.
    pub fn poll_decoded_images(&mut self, ctx: &egui::Context) {
        let pending: Vec<DecodeOutcome> = self.decoded_rx.try_iter().collect();
        for outcome in pending {
            self.pending_loads = self.pending_loads.saturating_sub(1);
            let img = match outcome {
                DecodeOutcome::Ok(img) => img,
                DecodeOutcome::Failed => continue,
            };
            let loaded = self.build_loaded_image(ctx, &img.name, img.file_size, img.size, img.rgba);
            self.set_side(img.side, loaded);
            if self.edge_detect {
                self.start_edge_compute(ctx);
            }
        }
    }
}

impl App {
    /// Clear both images, cached edges, and reset the view.
    fn clear_all(&mut self) {
        self.left = None;
        self.right = None;
        self.left_edge = None;
        self.right_edge = None;
        self.left_edge_key = None;
        self.right_edge_key = None;
        self.edge_detect = false;
        self.zoom = 1.0;
        self.pan_offset = Vec2::ZERO;
        self.separator = 0.5;
    }

    /// Wide-screen header: a single horizontal row with info on the left and
    /// controls aligned to the right.
    fn draw_header_wide(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.horizontal(|ui| {
            ui.heading("Image Compare");
            ui.separator();

            if let Some(left) = &self.left {
                ui.label(format!(
                    "Left: {} ({}) ({}x{})",
                    left.name,
                    format_file_size(left.file_size),
                    left.size[0],
                    left.size[1]
                ));
            }
            if let Some(right) = &self.right {
                ui.separator();
                ui.label(format!(
                    "Right: {} ({}) ({}x{})",
                    right.name,
                    format_file_size(right.file_size),
                    right.size[0],
                    right.size[1]
                ));
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(format!("Zoom: {:.0}%", self.zoom * 100.0));
                let toggle = ui.toggle_value(&mut self.edge_detect, "Edge Detect");
                if toggle.changed() && self.edge_detect {
                    self.start_edge_compute(ctx);
                }
                if ui.button("Reset").clicked() {
                    self.zoom = 1.0;
                    self.pan_offset = Vec2::ZERO;
                }
                if self.left.is_some() || self.right.is_some() {
                    if ui.button("Clear").clicked() {
                        self.clear_all();
                    }
                }
                if ui.button("Open Right").clicked() {
                    self.request_open(ctx, Side::Right);
                }
                if ui.button("Open Left").clicked() {
                    self.request_open(ctx, Side::Left);
                }
            });
        });
    }

    /// Narrow/mobile header: title and zoom on one row, touch-friendly controls
    /// that wrap onto multiple rows, and truncated image info below.
    fn draw_header_narrow(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        // Larger touch targets and spacing for finger input.
        let spacing = &mut ui.style_mut().spacing;
        spacing.button_padding = egui::vec2(10.0, 8.0);
        spacing.item_spacing = egui::vec2(8.0, 8.0);
        spacing.interact_size.y = spacing.interact_size.y.max(34.0);

        ui.add_space(2.0);
        ui.horizontal(|ui| {
            ui.heading("Image Compare");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(format!("{:.0}%", self.zoom * 100.0));
            });
        });

        // Controls wrap to additional rows when they don't fit.
        ui.horizontal_wrapped(|ui| {
            if ui.button("Open Left").clicked() {
                self.request_open(ctx, Side::Left);
            }
            if ui.button("Open Right").clicked() {
                self.request_open(ctx, Side::Right);
            }
            let toggle = ui.toggle_value(&mut self.edge_detect, "Edges");
            if toggle.changed() && self.edge_detect {
                self.start_edge_compute(ctx);
            }
            if ui.button("Reset").clicked() {
                self.zoom = 1.0;
                self.pan_offset = Vec2::ZERO;
            }
            if self.left.is_some() || self.right.is_some() {
                if ui.button("Clear").clicked() {
                    self.clear_all();
                }
            }
        });

        if let Some(left) = &self.left {
            ui.label(format!(
                "L: {} · {} · {}×{}",
                truncate_name(&left.name, 18),
                format_file_size(left.file_size),
                left.size[0],
                left.size[1]
            ));
        }
        if let Some(right) = &self.right {
            ui.label(format!(
                "R: {} · {} · {}×{}",
                truncate_name(&right.name, 18),
                format_file_size(right.file_size),
                right.size[0],
                right.size[1]
            ));
        }
        ui.add_space(2.0);
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_picked_files(ctx);
        self.poll_decoded_images(ctx);
        self.handle_dropped_files(ctx);

        ctx.input(|i| {
            if i.modifiers.ctrl && i.key_pressed(egui::Key::Num0) {
                self.zoom = 1.0;
                self.pan_offset = Vec2::ZERO;
            }
            if i.modifiers.ctrl && i.key_pressed(egui::Key::F) {
                self.zoom = 1.0;
                self.pan_offset = Vec2::ZERO;
            }
        });

        if ctx.input(|i| i.key_pressed(egui::Key::E)) {
            self.edge_detect = !self.edge_detect;
            if self.edge_detect {
                self.start_edge_compute(ctx);
            }
        }

        egui::TopBottomPanel::top("header").show(ctx, |ui| {
            // Switch to a compact, stacked layout on narrow (mobile) screens so
            // the controls and image info don't overflow off-screen.
            let narrow = ui.available_width() < 640.0;
            if narrow {
                self.draw_header_narrow(ui, ctx);
            } else {
                self.draw_header_wide(ui, ctx);
            }
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            if self.pending_loads > 0 {
                self.draw_loading(ui);
            } else if self.left.is_some() && self.right.is_some() {
                self.draw_comparison(ui);
            } else {
                self.draw_drop_zone(ui);
            }
        });
    }
}

/// Decode an encoded image using the browser's native decoder, then deliver the
/// raw RGBA8 pixels over `tx`. This handles every format the browser supports,
/// including PNG, JPEG, WebP, and AVIF.
///
/// The bytes are wrapped in a `Blob` and handed to `createImageBitmap`, which
/// decodes the image (potentially off the main thread). The resulting
/// `ImageBitmap` is drawn onto an offscreen canvas and read back via
/// `getImageData`.
fn decode_via_browser(
    ctx: egui::Context,
    tx: mpsc::Sender<DecodeOutcome>,
    side: Side,
    name: String,
    bytes: Vec<u8>,
) {
    use wasm_bindgen::JsCast;

    wasm_bindgen_futures::spawn_local(async move {
        let file_size = bytes.len();
        let t_start = now_ms();

        // Wrap the encoded bytes in a Blob.
        let array = js_sys::Uint8Array::from(bytes.as_slice());
        let parts = js_sys::Array::new();
        parts.push(&array);
        let Ok(blob) = web_sys::Blob::new_with_u8_array_sequence(&parts) else {
            let _ = tx.send(DecodeOutcome::Failed);
            ctx.request_repaint();
            return;
        };

        let log_name = name.clone();
        let decoded = async move {
            // `createImageBitmap` performs the decode, possibly off the main
            // thread, which is faster than going through an `HtmlImageElement`.
            let window = web_sys::window()?;
            let promise = window.create_image_bitmap_with_blob(&blob).ok()?;
            let bitmap: web_sys::ImageBitmap = wasm_bindgen_futures::JsFuture::from(promise)
                .await
                .ok()?
                .dyn_into()
                .ok()?;
            let t_decode = now_ms();

            let width = bitmap.width();
            let height = bitmap.height();
            if width == 0 || height == 0 {
                bitmap.close();
                return None;
            }

            // Draw onto an offscreen canvas and read the pixels back.
            let document = window.document()?;
            let canvas: web_sys::HtmlCanvasElement =
                document.create_element("canvas").ok()?.dyn_into().ok()?;
            canvas.set_width(width);
            canvas.set_height(height);
            let context: web_sys::CanvasRenderingContext2d =
                canvas.get_context("2d").ok()??.dyn_into().ok()?;
            let draw_result = context.draw_image_with_image_bitmap(&bitmap, 0.0, 0.0);
            // Release the bitmap's resources as soon as it has been drawn.
            bitmap.close();
            draw_result.ok()?;
            let t_draw = now_ms();
            let image_data = context
                .get_image_data(0.0, 0.0, width as f64, height as f64)
                .ok()?;
            let rgba = image_data.data().0;
            let t_readback = now_ms();

            log::info!(
                "[timing] decode '{}' ({}x{}, {} bytes): createImageBitmap {:.1}ms, draw_image {:.1}ms, get_image_data {:.1}ms, decode total {:.1}ms",
                log_name,
                width,
                height,
                file_size,
                t_decode - t_start,
                t_draw - t_decode,
                t_readback - t_draw,
                t_readback - t_start,
            );

            Some(DecodedImage {
                side,
                name,
                file_size,
                size: [width as usize, height as usize],
                rgba,
            })
        }
        .await;

        let outcome = match decoded {
            Some(decoded) => DecodeOutcome::Ok(decoded),
            None => DecodeOutcome::Failed,
        };
        if tx.send(outcome).is_ok() {
            // Wake the UI so the freshly decoded image (or cleared loading
            // state) is picked up.
            ctx.request_repaint();
        }
    });
}
