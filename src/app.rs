use eframe::egui;
use egui::{TextureHandle, TextureOptions, Vec2};
use std::sync::mpsc;

use crate::decode::{decode_via_browser, now_ms};
use crate::types::{DecodeOutcome, LoadedFile, LoadedImage, Side, NARROW_BREAKPOINT};

/// Returns a process-unique, monotonically increasing id for each loaded image.
/// Used as a cheap cache key for edge detection instead of hashing every pixel.
fn next_image_id() -> u64 {
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
    /// Reset zoom and pan to their defaults, leaving the separator untouched.
    pub fn reset_zoom_pan(&mut self) {
        self.zoom = 1.0;
        self.pan_offset = Vec2::ZERO;
    }
}

/// Edge-detection toggle plus cached edge textures for each side.
#[derive(Default)]
pub struct EdgeState {
    pub enabled: bool,
    pub left: Option<TextureHandle>,
    pub right: Option<TextureHandle>,
    /// Id of the image the cached `left` texture was computed from.
    pub left_key: Option<u64>,
    /// Id of the image the cached `right` texture was computed from.
    pub right_key: Option<u64>,
}

/// Channels and counters coordinating asynchronous file picking and decoding.
pub struct AsyncIo {
    pub file_tx: mpsc::Sender<LoadedFile>,
    pub file_rx: mpsc::Receiver<LoadedFile>,
    pub decoded_tx: mpsc::Sender<DecodeOutcome>,
    pub decoded_rx: mpsc::Receiver<DecodeOutcome>,
    /// Number of images currently being decoded asynchronously. While > 0 a
    /// loading screen is shown instead of the drop zones.
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
    /// Number of files in the current/most recent drag gesture (known during
    /// dragover, before the drop's bytes arrive).
    pub expected: usize,
    /// How many files of the current drop gesture have been routed so far.
    pub index: usize,
}

#[derive(Default)]
pub struct App {
    pub left: Option<LoadedImage>,
    pub right: Option<LoadedImage>,
    pub view: ViewState,
    pub edges: EdgeState,
    pub io: AsyncIo,
    pub drop: DropState,
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
                self.edges.left = None;
                self.edges.left_key = None;
            }
            Side::Right => {
                self.right = Some(loaded);
                self.edges.right = None;
                self.edges.right_key = None;
            }
        }
    }

    /// Decode `bytes` and assign the result to `side`. Decoding is delegated to
    /// the browser's native image decoder and arrives asynchronously via
    /// `decoded_rx`.
    fn load_and_set(&mut self, ctx: &egui::Context, side: Side, name: &str, bytes: &[u8]) {
        self.io.pending_loads += 1;
        decode_via_browser(
            ctx.clone(),
            self.io.decoded_tx.clone(),
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
            if !self.drop.was_hovering {
                // A new drag gesture started: reset the per-drop routing index.
                self.drop.index = 0;
            }
            // Number of files being dragged, available during dragover.
            self.drop.expected = hovering_count;
        }
        self.drop.was_hovering = hovering_count > 0;

        let dropped_files: Vec<(String, Vec<u8>)> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.bytes.as_ref().map(|b| (f.name.clone(), b.to_vec())))
                .collect()
        });

        for (name, bytes) in dropped_files {
            let side = if self.drop.expected >= 2 {
                // Multi-file drop: first file -> left, second -> right, ignore
                // any extras.
                match self.drop.index {
                    0 => Side::Left,
                    1 => Side::Right,
                    _ => {
                        self.drop.index += 1;
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
            self.drop.index += 1;
            self.load_and_set(ctx, side, &name, &bytes);
        }
    }

    /// Open the native browser file picker directly (no confirmation dialog) and
    /// deliver the picked file's bytes over a channel.
    pub fn request_open(&self, ctx: &egui::Context, side: Side) {
        use wasm_bindgen::JsCast;
        use wasm_bindgen::closure::Closure;

        let tx = self.io.file_tx.clone();
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
                if tx
                    .send(LoadedFile {
                        side: Some(side),
                        name,
                        bytes,
                    })
                    .is_ok()
                {
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

    /// Install a one-shot document `paste` listener that loads the first image
    /// from the clipboard through the same channel as the file picker.
    ///
    /// Safe to call every frame: registration is guarded by a process-wide
    /// `Once` so the listener is only attached once.
    pub fn install_paste_listener(&self, ctx: &egui::Context) {
        use std::sync::Once;
        use wasm_bindgen::JsCast;
        use wasm_bindgen::closure::Closure;

        static PASTE_LISTENER: Once = Once::new();
        let tx = self.io.file_tx.clone();
        let ctx = ctx.clone();

        PASTE_LISTENER.call_once(move || {
            let Some(window) = web_sys::window() else {
                return;
            };
            let Some(document) = window.document() else {
                return;
            };

            let closure = Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
                let Ok(clipboard_event) = event.dyn_into::<web_sys::ClipboardEvent>() else {
                    return;
                };
                let Some(data) = clipboard_event.clipboard_data() else {
                    return;
                };
                let items = data.items();
                for i in 0..items.length() {
                    let Some(item) = items.get(i) else {
                        continue;
                    };
                    let mime = item.type_();
                    if !mime.starts_with("image/") {
                        continue;
                    }
                    let Ok(Some(file)) = item.get_as_file() else {
                        continue;
                    };

                    // Stop the browser from also handling this paste.
                    clipboard_event.prevent_default();

                    let name = {
                        let n = file.name();
                        if n.is_empty() {
                            // Screenshots and some OS pastes omit a filename.
                            let ext = mime.rsplit('/').next().unwrap_or("png");
                            // Normalize common MIME subtypes to file extensions.
                            let ext = match ext {
                                "jpeg" => "jpg",
                                "svg+xml" => "svg",
                                other => other,
                            };
                            format!("clipboard.{ext}")
                        } else {
                            n
                        }
                    };
                    let tx = tx.clone();
                    let ctx = ctx.clone();
                    wasm_bindgen_futures::spawn_local(async move {
                        let Ok(buffer) =
                            wasm_bindgen_futures::JsFuture::from(file.array_buffer()).await
                        else {
                            return;
                        };
                        let bytes = js_sys::Uint8Array::new(&buffer).to_vec();
                        if tx
                            .send(LoadedFile {
                                side: None,
                                name,
                                bytes,
                            })
                            .is_ok()
                        {
                            ctx.request_repaint();
                        }
                    });
                    // Only the first image in the clipboard is used.
                    break;
                }
            });

            let _ = document.add_event_listener_with_callback(
                "paste",
                closure.as_ref().unchecked_ref(),
            );
            // Keep the closure alive for the lifetime of the page.
            closure.forget();
        });
    }

    /// Drain any files picked through the async dialog (or pasted) and apply them.
    pub fn poll_picked_files(&mut self, ctx: &egui::Context) {
        let pending: Vec<LoadedFile> = self.io.file_rx.try_iter().collect();
        for file in pending {
            let side = file.side.unwrap_or_else(|| {
                if self.left.is_none() {
                    Side::Left
                } else {
                    Side::Right
                }
            });
            self.load_and_set(ctx, side, &file.name, &file.bytes);
        }
    }

    /// Drain any images decoded asynchronously by the browser (e.g. AVIF) and
    /// apply them to their target side.
    pub fn poll_decoded_images(&mut self, ctx: &egui::Context) {
        let pending: Vec<DecodeOutcome> = self.io.decoded_rx.try_iter().collect();
        for outcome in pending {
            self.io.pending_loads = self.io.pending_loads.saturating_sub(1);
            let img = match outcome {
                DecodeOutcome::Ok(img) => img,
                DecodeOutcome::Failed => continue,
            };
            let loaded = self.build_loaded_image(ctx, &img.name, img.file_size, img.size, img.rgba);
            self.set_side(img.side, loaded);
            if self.edges.enabled {
                self.start_edge_compute(ctx);
            }
        }
    }

    /// Clear both images, cached edges, and reset the view.
    pub fn clear_all(&mut self) {
        self.left = None;
        self.right = None;
        self.edges = EdgeState::default();
        self.view = ViewState::default();
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.install_paste_listener(ctx);
        self.poll_picked_files(ctx);
        self.poll_decoded_images(ctx);
        self.handle_dropped_files(ctx);

        ctx.input(|i| {
            // Ctrl+0 and Ctrl+F both reset zoom and pan to fit.
            let reset = i.modifiers.ctrl
                && (i.key_pressed(egui::Key::Num0) || i.key_pressed(egui::Key::F));
            if reset {
                self.view.reset_zoom_pan();
            }
        });

        if ctx.input(|i| i.key_pressed(egui::Key::E)) {
            self.edges.enabled = !self.edges.enabled;
            if self.edges.enabled {
                self.start_edge_compute(ctx);
            }
        }

        egui::TopBottomPanel::top("header").show(ctx, |ui| {
            // Switch to a compact, stacked layout on narrow (mobile) screens so
            // the controls and image info don't overflow off-screen.
            let narrow = ui.available_width() < NARROW_BREAKPOINT;
            if narrow {
                self.draw_header_narrow(ui, ctx);
            } else {
                self.draw_header_wide(ui, ctx);
            }
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            if self.io.pending_loads > 0 {
                self.draw_loading(ui);
            } else if self.left.is_some() && self.right.is_some() {
                self.draw_comparison(ui);
            } else {
                self.draw_drop_zone(ui);
            }
        });
    }
}
