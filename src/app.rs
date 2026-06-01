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
    /// Hash of the raw pixels, used to cache edge detection results.
    pub hash: u64,
}

/// Compute a fast content hash of raw pixel data.
fn hash_rgba(rgba: &[u8]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    rgba.hash(&mut hasher);
    hasher.finish()
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
        let color_image = egui::ColorImage::from_rgba_unmultiplied(size, &raw);
        let texture = ctx.load_texture(name, color_image, TextureOptions::LINEAR);
        let hash = hash_rgba(&raw);

        LoadedImage {
            texture,
            name: name.to_string(),
            size,
            file_size,
            rgba: raw,
            hash,
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
    pub fn request_open(&self, side: Side) {
        use wasm_bindgen::JsCast;
        use wasm_bindgen::closure::Closure;

        let tx = self.file_tx.clone();
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
            wasm_bindgen_futures::spawn_local(async move {
                let Ok(buffer) = wasm_bindgen_futures::JsFuture::from(file.array_buffer()).await
                else {
                    return;
                };
                let bytes = js_sys::Uint8Array::new(&buffer).to_vec();
                let _ = tx.send(LoadedFile { side, name, bytes });
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
                    }
                    if ui.button("Open Right").clicked() {
                        self.request_open(Side::Right);
                    }
                    if ui.button("Open Left").clicked() {
                        self.request_open(Side::Left);
                    }
                });
            });
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
/// The bytes are wrapped in a `Blob`, loaded into an `HtmlImageElement`, drawn
/// onto an offscreen canvas, and read back via `getImageData`.
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

        // Wrap the encoded bytes in a Blob and hand the browser an object URL.
        let array = js_sys::Uint8Array::from(bytes.as_slice());
        let parts = js_sys::Array::new();
        parts.push(&array);
        let Ok(blob) = web_sys::Blob::new_with_u8_array_sequence(&parts) else {
            let _ = tx.send(DecodeOutcome::Failed);
            ctx.request_repaint();
            return;
        };
        let Ok(url) = web_sys::Url::create_object_url_with_blob(&blob) else {
            let _ = tx.send(DecodeOutcome::Failed);
            ctx.request_repaint();
            return;
        };

        let url_for_revoke = url.clone();
        let decoded = async move {
            let img = web_sys::HtmlImageElement::new().ok()?;
            img.set_src(&url);
            wasm_bindgen_futures::JsFuture::from(img.decode())
                .await
                .ok()?;

            let width = img.natural_width();
            let height = img.natural_height();
            if width == 0 || height == 0 {
                return None;
            }

            // Draw onto an offscreen canvas and read the pixels back.
            let document = web_sys::window()?.document()?;
            let canvas: web_sys::HtmlCanvasElement =
                document.create_element("canvas").ok()?.dyn_into().ok()?;
            canvas.set_width(width);
            canvas.set_height(height);
            let context: web_sys::CanvasRenderingContext2d =
                canvas.get_context("2d").ok()??.dyn_into().ok()?;
            context
                .draw_image_with_html_image_element(&img, 0.0, 0.0)
                .ok()?;
            let image_data = context
                .get_image_data(0.0, 0.0, width as f64, height as f64)
                .ok()?;

            Some(DecodedImage {
                side,
                name,
                file_size,
                size: [width as usize, height as usize],
                rgba: image_data.data().0,
            })
        }
        .await;
        let _ = web_sys::Url::revoke_object_url(&url_for_revoke);

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
