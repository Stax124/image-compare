use eframe::egui;

use crate::state::{AsyncIo, DropState, EdgeState, ViewState};
use crate::types::{LoadedImage, NARROW_BREAKPOINT, Side};

#[derive(Default)]
pub struct App {
    pub images: [Option<LoadedImage>; 2],
    pub view: ViewState,
    pub edges: EdgeState,
    pub io: AsyncIo,
    pub drop: DropState,
}

impl App {
    pub fn image(&self, side: Side) -> Option<&LoadedImage> {
        self.images[side.index()].as_ref()
    }

    /// First empty side, or right if both are filled.
    pub fn auto_route_side(&self) -> Side {
        if self.images[Side::Left.index()].is_none() {
            Side::Left
        } else {
            Side::Right
        }
    }

    /// Both comparison slots are filled.
    pub fn both_loaded(&self) -> bool {
        self.images.iter().all(|img| img.is_some())
    }

    /// At least one image is loaded.
    pub fn any_loaded(&self) -> bool {
        self.images.iter().any(|img| img.is_some())
    }

    /// Assign a loaded image to one side, clearing that side's cached edges.
    pub fn set_image(&mut self, side: Side, loaded: LoadedImage) {
        self.images[side.index()] = Some(loaded);
        self.edges.clear_side(side);
    }

    pub fn set_edges_enabled(&mut self, ctx: &egui::Context, enabled: bool) {
        self.edges.enabled = enabled;
        if enabled {
            self.start_edge_compute(ctx);
        }
    }

    /// Clear both images, cached edges, and reset the view.
    pub fn clear_all(&mut self) {
        self.images = [None, None];
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
            let reset =
                i.modifiers.ctrl && (i.key_pressed(egui::Key::Num0) || i.key_pressed(egui::Key::F));
            if reset {
                self.view.reset_zoom_pan();
            }
        });

        if ctx.input(|i| i.key_pressed(egui::Key::E)) {
            self.set_edges_enabled(ctx, !self.edges.enabled);
        }

        egui::TopBottomPanel::top("header").show(ctx, |ui| {
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
            } else if self.both_loaded() {
                self.draw_comparison(ui);
            } else {
                self.draw_drop_zone(ui);
            }
        });
    }
}
