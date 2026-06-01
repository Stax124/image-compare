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
    /// Raw RGBA8 pixels, kept in memory so edge detection can run without disk access.
    pub rgba: Vec<u8>,
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

    /// Open the browser file picker asynchronously and deliver the bytes over a channel.
    pub fn request_open(&self, side: Side) {
        let tx = self.file_tx.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let file = rfd::AsyncFileDialog::new()
                .add_filter("Images", &["png", "jpg", "jpeg", "webp", "bmp", "tiff"])
                .pick_file()
                .await;

            if let Some(file) = file {
                let name = file.file_name();
                let bytes = file.read().await;
                let _ = tx.send(LoadedFile { side, name, bytes });
            }
        });
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
                        "Left: {} ({}x{})",
                        left.name, left.size[0], left.size[1]
                    ));
                }
                if let Some(right) = &self.right {
                    ui.separator();
                    ui.label(format!(
                        "Right: {} ({}x{})",
                        right.name, right.size[0], right.size[1]
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
