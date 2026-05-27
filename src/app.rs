use eframe::egui;
use egui::{TextureHandle, TextureOptions, Vec2};
use std::path::PathBuf;
use std::sync::mpsc;

use crate::edge::EdgeResult;

pub struct LoadedImage {
    pub texture: TextureHandle,
    pub name: String,
    pub size: [usize; 2],
    pub path: PathBuf,
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
    pub edge_computing: bool,
    pub left_edge: Option<TextureHandle>,
    pub right_edge: Option<TextureHandle>,
    pub edge_rx: Option<mpsc::Receiver<EdgeResult>>,
}

impl Default for App {
    fn default() -> Self {
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
            edge_computing: false,
            left_edge: None,
            right_edge: None,
            edge_rx: None,
        }
    }
}

impl App {
    pub fn load_image(&self, ctx: &egui::Context, path: &PathBuf) -> Option<LoadedImage> {
        let img = image::open(path).ok()?;
        let rgba = img.to_rgba8();
        let size = [rgba.width() as usize, rgba.height() as usize];
        let color_image = egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());

        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "unknown".to_string());

        let texture = ctx.load_texture(&name, color_image, TextureOptions::LINEAR);

        Some(LoadedImage {
            texture,
            name,
            size,
            path: path.clone(),
        })
    }

    pub fn handle_dropped_files(&mut self, ctx: &egui::Context) {
        let dropped_files: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.path.clone())
                .collect()
        });

        for path in dropped_files {
            if let Some(loaded) = self.load_image(ctx, &path) {
                if self.left.is_none() {
                    self.left = Some(loaded);
                    self.left_edge = None;
                } else if self.right.is_none() {
                    self.right = Some(loaded);
                    self.right_edge = None;
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

    pub fn pick_image_file() -> Option<PathBuf> {
        rfd::FileDialog::new()
            .add_filter("Images", &["png", "jpg", "jpeg", "webp", "bmp", "tiff"])
            .pick_file()
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_edge_results(ctx);
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
                    let edge_label = if self.edge_computing {
                        "Edge Detect ⏳"
                    } else {
                        "Edge Detect"
                    };
                    let toggle = ui.toggle_value(&mut self.edge_detect, edge_label);
                    if toggle.changed() {
                        if self.edge_detect {
                            self.start_edge_compute(ctx);
                        } else {
                            self.left_edge = None;
                            self.right_edge = None;
                            self.edge_computing = false;
                            self.edge_rx = None;
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
                        if let Some(path) = App::pick_image_file() {
                            if let Some(loaded) = self.load_image(ui.ctx(), &path) {
                                self.right = Some(loaded);
                            }
                        }
                    }
                    if ui.button("Open Left").clicked() {
                        if let Some(path) = App::pick_image_file() {
                            if let Some(loaded) = self.load_image(ui.ctx(), &path) {
                                self.left = Some(loaded);
                            }
                        }
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
