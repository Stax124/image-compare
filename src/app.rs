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

pub struct LoadedImage {
    pub texture: TextureHandle,
    pub name: String,
    pub size: [usize; 2],
    /// Size of the original encoded file in bytes.
    pub file_size: usize,
    /// Raw RGBA8 pixels, kept in memory so edge detection can run without disk access.
    pub rgba: Vec<u8>,
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
    pub file_tx: mpsc::Sender<LoadedFile>,
    pub file_rx: mpsc::Receiver<LoadedFile>,
}

impl Default for App {
    fn default() -> Self {
        let (file_tx, file_rx) = mpsc::channel();
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
            file_tx,
            file_rx,
        }
    }
}

impl App {
    pub fn load_image(&self, ctx: &egui::Context, name: &str, bytes: &[u8]) -> Option<LoadedImage> {
        let img = image::load_from_memory(bytes).ok()?;
        let rgba = img.to_rgba8();
        let size = [rgba.width() as usize, rgba.height() as usize];
        let raw = rgba.into_raw();
        let color_image = egui::ColorImage::from_rgba_unmultiplied(size, &raw);

        let texture = ctx.load_texture(name, color_image, TextureOptions::LINEAR);

        Some(LoadedImage {
            texture,
            name: name.to_string(),
            size,
            file_size: bytes.len(),
            rgba: raw,
        })
    }

    pub fn handle_dropped_files(&mut self, ctx: &egui::Context) {
        let dropped_files: Vec<(String, Vec<u8>)> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.bytes.as_ref().map(|b| (f.name.clone(), b.to_vec())))
                .collect()
        });

        if dropped_files.len() >= 2 {
            // Multiple images dropped at once: overwrite both zones with the
            // first two, ignoring any extras.
            let mut changed = false;
            for ((name, bytes), side) in dropped_files.iter().take(2).zip([Side::Left, Side::Right])
            {
                if let Some(loaded) = self.load_image(ctx, name, bytes) {
                    match side {
                        Side::Left => {
                            self.left = Some(loaded);
                            self.left_edge = None;
                        }
                        Side::Right => {
                            self.right = Some(loaded);
                            self.right_edge = None;
                        }
                    }
                    changed = true;
                }
            }
            if changed && self.edge_detect {
                self.start_edge_compute(ctx);
            }
            return;
        }

        for (name, bytes) in dropped_files {
            if let Some(loaded) = self.load_image(ctx, &name, &bytes) {
                if self.left.is_none() {
                    self.left = Some(loaded);
                    self.left_edge = None;
                } else {
                    self.right = Some(loaded);
                    self.right_edge = None;
                }
                if self.edge_detect {
                    self.start_edge_compute(ctx);
                }
            }
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
            "image/png,image/jpeg,image/webp,image/bmp,image/tiff,.png,.jpg,.jpeg,.webp,.bmp,.tiff",
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
            if let Some(loaded) = self.load_image(ctx, &file.name, &file.bytes) {
                match file.side {
                    Side::Left => {
                        self.left = Some(loaded);
                        self.left_edge = None;
                    }
                    Side::Right => {
                        self.right = Some(loaded);
                        self.right_edge = None;
                    }
                }
                if self.edge_detect {
                    self.start_edge_compute(ctx);
                }
            }
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_picked_files(ctx);
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
                    if toggle.changed() {
                        if self.edge_detect {
                            self.start_edge_compute(ctx);
                        } else {
                            self.left_edge = None;
                            self.right_edge = None;
                        }
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
            if self.left.is_some() && self.right.is_some() {
                self.draw_comparison(ui);
            } else {
                self.draw_drop_zone(ui);
            }
        });
    }
}
