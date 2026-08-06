use eframe::egui;
use egui::{Color32, Pos2, Rect, Sense, Stroke, Vec2};

use crate::app::App;
use crate::types::{NARROW_BREAKPOINT, Side};

/// Format a byte count into a human-readable string (e.g. "1.2 MB").
fn format_file_size(bytes: usize) -> String {
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

impl App {
    pub fn fit_image_rect(&self, canvas: Rect, img_size: [usize; 2]) -> Rect {
        let img_w = img_size[0] as f32;
        let img_h = img_size[1] as f32;
        let canvas_w = canvas.width();
        let canvas_h = canvas.height();

        let scale = (canvas_w / img_w).min(canvas_h / img_h) * self.view.zoom;

        let display_w = img_w * scale;
        let display_h = img_h * scale;

        let center = canvas.center() + self.view.pan_offset;

        Rect::from_center_size(center, Vec2::new(display_w, display_h))
    }

    pub fn draw_comparison(&mut self, ui: &mut egui::Ui) {
        let available = ui.available_size();
        let (response, painter) = ui.allocate_painter(available, Sense::click_and_drag());
        let canvas = response.rect;
        let separator_x = canvas.left() + canvas.width() * self.view.separator;

        self.draw_images(&painter, canvas, separator_x);
        self.draw_separator(&painter, canvas, separator_x);

        let pointer_pos = ui.input(|i| i.pointer.hover_pos());
        self.handle_separator_drag(ui, &response, canvas, pointer_pos);
        self.handle_zoom(ui, &response, canvas, pointer_pos);
    }

    /// Draw both images, each clipped to its half of the separator. When edge
    /// detection is on, the cached edge texture is used if available.
    fn draw_images(&self, painter: &egui::Painter, canvas: Rect, separator_x: f32) {
        let left = self.left.as_ref().unwrap();
        let right = self.right.as_ref().unwrap();

        let tex_id = |edge: &Option<egui::TextureHandle>, base: &egui::TextureHandle| {
            if self.edges.enabled {
                edge.as_ref().map(|t| t.id()).unwrap_or_else(|| base.id())
            } else {
                base.id()
            }
        };
        let left_tex_id = tex_id(&self.edges.left, &left.texture);
        let right_tex_id = tex_id(&self.edges.right, &right.texture);

        let uv = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0));

        // Left image (clipped to left of separator).
        let left_clip =
            Rect::from_min_max(canvas.left_top(), Pos2::new(separator_x, canvas.bottom()));
        let left_rect = self.fit_image_rect(canvas, left.size);
        painter
            .with_clip_rect(left_clip)
            .image(left_tex_id, left_rect, uv, Color32::WHITE);

        // Right image (clipped to right of separator).
        let right_clip =
            Rect::from_min_max(Pos2::new(separator_x, canvas.top()), canvas.right_bottom());
        let right_rect = self.fit_image_rect(canvas, right.size);
        painter
            .with_clip_rect(right_clip)
            .image(right_tex_id, right_rect, uv, Color32::WHITE);
    }

    /// Draw the vertical separator line and its centered drag handle.
    fn draw_separator(&self, painter: &egui::Painter, canvas: Rect, separator_x: f32) {
        let line_top = Pos2::new(separator_x, canvas.top());
        let line_bottom = Pos2::new(separator_x, canvas.bottom());

        // Shadow behind a white line.
        painter.line_segment(
            [line_top, line_bottom],
            Stroke::new(4.0, Color32::from_black_alpha(100)),
        );
        painter.line_segment([line_top, line_bottom], Stroke::new(2.0, Color32::WHITE));

        // Handle circle at the vertical center.
        let handle_center = Pos2::new(separator_x, canvas.center().y);
        painter.circle_filled(handle_center, 12.0, Color32::from_black_alpha(160));
        painter.circle_stroke(handle_center, 12.0, Stroke::new(2.0, Color32::WHITE));

        // Left/right arrows on the handle.
        let stroke = Stroke::new(2.0, Color32::WHITE);
        let offset = 5.0;
        for dir in [-1.0_f32, 1.0] {
            let tip = handle_center + Vec2::new(dir * offset, 0.0);
            let base_x = dir * (offset - 3.0);
            painter.line_segment([tip, handle_center + Vec2::new(base_x, 3.0)], stroke);
            painter.line_segment([tip, handle_center + Vec2::new(base_x, -3.0)], stroke);
        }
    }

    /// Handle separator dragging and panning, updating the cursor icon while the
    /// pointer is over the separator.
    fn handle_separator_drag(
        &mut self,
        ui: &mut egui::Ui,
        response: &egui::Response,
        canvas: Rect,
        pointer_pos: Option<Pos2>,
    ) {
        let separator_x = canvas.left() + canvas.width() * self.view.separator;
        let separator_hit_rect = Rect::from_center_size(
            Pos2::new(separator_x, canvas.center().y),
            Vec2::new(50.0, canvas.height()),
        );
        let pointer_in_separator = pointer_pos
            .map(|p| separator_hit_rect.contains(p))
            .unwrap_or(false);

        if pointer_in_separator || self.view.dragging_separator {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        }

        if response.drag_started()
            && let Some(pos) = pointer_pos
        {
            if separator_hit_rect.contains(pos) {
                self.view.dragging_separator = true;
            } else {
                self.view.panning = true;
                self.view.last_pan_pos = Some(pos);
            }
        }

        if response.dragged() {
            if self.view.dragging_separator {
                if let Some(pos) = pointer_pos {
                    self.view.separator =
                        ((pos.x - canvas.left()) / canvas.width()).clamp(0.01, 0.99);
                }
            } else if self.view.panning
                && let Some(pos) = pointer_pos
            {
                if let Some(last) = self.view.last_pan_pos {
                    self.view.pan_offset += pos - last;
                }
                self.view.last_pan_pos = Some(pos);
            }
        }

        if response.drag_stopped() {
            self.view.dragging_separator = false;
            self.view.panning = false;
            self.view.last_pan_pos = None;
        }
    }

    /// Handle scroll-wheel and pinch zoom, keeping the point under the cursor
    /// fixed in place.
    fn handle_zoom(
        &mut self,
        ui: &mut egui::Ui,
        response: &egui::Response,
        canvas: Rect,
        pointer_pos: Option<Pos2>,
    ) {
        if !response.hovered() {
            return;
        }

        let scroll_delta = ui.input(|i| i.smooth_scroll_delta.y);
        if scroll_delta != 0.0 {
            let new_zoom = (self.view.zoom * (1.0 + scroll_delta * 0.002)).clamp(0.1, 50.0);
            self.apply_zoom_at(new_zoom, pointer_pos, canvas);
        }

        let multi_zoom = ui.input(|i| i.zoom_delta());
        if multi_zoom != 1.0 {
            let new_zoom = (self.view.zoom * multi_zoom).clamp(0.1, 50.0);
            self.apply_zoom_at(new_zoom, pointer_pos, canvas);
        }
    }

    /// Set the zoom level while adjusting the pan so the point under `cursor`
    /// stays put.
    fn apply_zoom_at(&mut self, new_zoom: f32, cursor: Option<Pos2>, canvas: Rect) {
        if let Some(cursor) = cursor {
            let cursor_rel = cursor - (canvas.center() + self.view.pan_offset);
            self.view.pan_offset -= cursor_rel * (new_zoom / self.view.zoom - 1.0);
        }
        self.view.zoom = new_zoom;
    }

    /// Wide-screen header: a single horizontal row with info on the left and
    /// controls aligned to the right.
    pub fn draw_header_wide(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
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
                ui.label(format!("Zoom: {:.0}%", self.view.zoom * 100.0));
                let toggle = ui.toggle_value(&mut self.edges.enabled, "Edge Detect");
                if toggle.changed() && self.edges.enabled {
                    self.start_edge_compute(ctx);
                }
                if ui.button("Reset").clicked() {
                    self.view.reset_zoom_pan();
                }
                if (self.left.is_some() || self.right.is_some()) && ui.button("Clear").clicked() {
                    self.clear_all();
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
    pub fn draw_header_narrow(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        // Larger touch targets and spacing for finger input.
        let spacing = &mut ui.style_mut().spacing;
        spacing.button_padding = egui::vec2(10.0, 8.0);
        spacing.item_spacing = egui::vec2(8.0, 8.0);
        spacing.interact_size.y = spacing.interact_size.y.max(34.0);

        ui.add_space(2.0);
        ui.horizontal(|ui| {
            ui.heading("Image Compare");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(format!("{:.0}%", self.view.zoom * 100.0));
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
            let toggle = ui.toggle_value(&mut self.edges.enabled, "Edges");
            if toggle.changed() && self.edges.enabled {
                self.start_edge_compute(ctx);
            }
            if ui.button("Reset").clicked() {
                self.view.reset_zoom_pan();
            }
            if (self.left.is_some() || self.right.is_some()) && ui.button("Clear").clicked() {
                self.clear_all();
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

    pub fn draw_loading(&mut self, ui: &mut egui::Ui) {
        let available = ui.available_size();
        let (response, painter) = ui.allocate_painter(available, Sense::hover());
        let rect = response.rect;

        painter.rect_filled(rect, 0.0, Color32::from_gray(30));

        // Spinning arc indicator.
        let center = rect.center() - Vec2::new(0.0, 24.0);
        let radius = 22.0;
        let t = ui.input(|i| i.time) as f32;
        let start = t * 4.0;
        let segments = 24;
        let sweep = std::f32::consts::PI * 1.4;
        let mut prev: Option<Pos2> = None;
        for k in 0..=segments {
            let a = start + sweep * (k as f32 / segments as f32);
            let p = center + Vec2::new(a.cos(), a.sin()) * radius;
            if let Some(prev) = prev {
                painter.line_segment(
                    [prev, p],
                    Stroke::new(3.0, Color32::from_rgb(100, 160, 255)),
                );
            }
            prev = Some(p);
        }

        painter.text(
            center + Vec2::new(0.0, radius + 28.0),
            egui::Align2::CENTER_CENTER,
            "Loading…",
            egui::FontId::proportional(24.0),
            Color32::from_gray(200),
        );

        // Keep animating while decoding completes.
        ui.ctx().request_repaint();
    }

    pub fn draw_drop_zone(&mut self, ui: &mut egui::Ui) {
        let available = ui.available_size();
        let (response, painter) = ui.allocate_painter(available, Sense::hover());
        let rect = response.rect;

        let is_hovering = ui.input(|i| !i.raw.hovered_files.is_empty());

        let bg_color = if is_hovering {
            Color32::from_rgba_premultiplied(60, 80, 120, 255)
        } else {
            Color32::from_gray(30)
        };
        painter.rect_filled(rect, 0.0, bg_color);

        // Dashed border that highlights while files are dragged over it.
        let border_color = if is_hovering {
            Color32::from_rgb(100, 160, 255)
        } else {
            Color32::from_gray(80)
        };
        let inset = rect.shrink(20.0);
        painter.rect_stroke(
            inset,
            12.0,
            Stroke::new(2.0, border_color),
            egui::StrokeKind::Middle,
        );

        let text = if self.left.is_none() {
            "First image"
        } else {
            "Second image"
        };

        let narrow = rect.width() < NARROW_BREAKPOINT;
        let title_size = if narrow { 22.0 } else { 28.0 };
        let hint_size = if narrow { 14.0 } else { 18.0 };
        let gap = title_size * 0.5 + hint_size * 0.5 + 6.0;

        painter.text(
            rect.center() - Vec2::new(0.0, gap),
            egui::Align2::CENTER_CENTER,
            text,
            egui::FontId::proportional(title_size),
            Color32::from_gray(200),
        );

        painter.text(
            rect.center() + Vec2::new(0.0, gap),
            egui::Align2::CENTER_CENTER,
            "Drag & drop, paste, or tap to browse",
            egui::FontId::proportional(hint_size),
            Color32::from_gray(150),
        );

        // Click to open the file picker for the next empty side.
        if response.interact(Sense::click()).clicked() {
            let side = if self.left.is_none() {
                Side::Left
            } else {
                Side::Right
            };
            self.request_open(ui.ctx(), side);
        }
    }
}
