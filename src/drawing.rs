use eframe::egui;
use egui::{Color32, Pos2, Rect, Sense, Stroke, Vec2};

use crate::app::App;

impl App {
    pub fn fit_image_rect(&self, canvas: Rect, img_size: [usize; 2]) -> Rect {
        let img_w = img_size[0] as f32;
        let img_h = img_size[1] as f32;
        let canvas_w = canvas.width();
        let canvas_h = canvas.height();

        let scale = (canvas_w / img_w).min(canvas_h / img_h) * self.zoom;

        let display_w = img_w * scale;
        let display_h = img_h * scale;

        let center = canvas.center() + self.pan_offset;

        Rect::from_center_size(center, Vec2::new(display_w, display_h))
    }

    pub fn draw_comparison(&mut self, ui: &mut egui::Ui) {
        let available = ui.available_size();
        let (response, painter) = ui.allocate_painter(available, Sense::click_and_drag());
        let canvas = response.rect;

        let left = self.left.as_ref().unwrap();
        let right = self.right.as_ref().unwrap();

        let left_tex_id = if self.edge_detect {
            self.left_edge
                .as_ref()
                .map(|t| t.id())
                .unwrap_or_else(|| left.texture.id())
        } else {
            left.texture.id()
        };
        let right_tex_id = if self.edge_detect {
            self.right_edge
                .as_ref()
                .map(|t| t.id())
                .unwrap_or_else(|| right.texture.id())
        } else {
            right.texture.id()
        };

        let separator_x = canvas.left() + canvas.width() * self.separator;

        // Left image (clipped to left of separator)
        let left_clip =
            Rect::from_min_max(canvas.left_top(), Pos2::new(separator_x, canvas.bottom()));
        let left_painter = painter.with_clip_rect(left_clip);
        let left_rect = self.fit_image_rect(canvas, left.size);
        left_painter.image(
            left_tex_id,
            left_rect,
            Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0)),
            Color32::WHITE,
        );

        // Right image (clipped to right of separator)
        let right_clip =
            Rect::from_min_max(Pos2::new(separator_x, canvas.top()), canvas.right_bottom());
        let right_painter = painter.with_clip_rect(right_clip);
        let right_rect = self.fit_image_rect(canvas, right.size);
        right_painter.image(
            right_tex_id,
            right_rect,
            Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0)),
            Color32::WHITE,
        );

        // Draw separator line
        let line_top = Pos2::new(separator_x, canvas.top());
        let line_bottom = Pos2::new(separator_x, canvas.bottom());

        // Shadow
        painter.line_segment(
            [line_top, line_bottom],
            Stroke::new(4.0, Color32::from_black_alpha(100)),
        );
        // White line
        painter.line_segment([line_top, line_bottom], Stroke::new(2.0, Color32::WHITE));

        // Draw handle circle at center of separator
        let handle_center = Pos2::new(separator_x, canvas.center().y);
        painter.circle_filled(handle_center, 12.0, Color32::from_black_alpha(160));
        painter.circle_stroke(handle_center, 12.0, Stroke::new(2.0, Color32::WHITE));

        // Draw arrows on handle
        let arrow_offset = 5.0;
        // Left arrow
        painter.line_segment(
            [
                handle_center - Vec2::new(arrow_offset, 0.0),
                handle_center - Vec2::new(arrow_offset - 3.0, 3.0),
            ],
            Stroke::new(2.0, Color32::WHITE),
        );
        painter.line_segment(
            [
                handle_center - Vec2::new(arrow_offset, 0.0),
                handle_center - Vec2::new(arrow_offset - 3.0, -3.0),
            ],
            Stroke::new(2.0, Color32::WHITE),
        );
        // Right arrow
        painter.line_segment(
            [
                handle_center + Vec2::new(arrow_offset, 0.0),
                handle_center + Vec2::new(arrow_offset - 3.0, 3.0),
            ],
            Stroke::new(2.0, Color32::WHITE),
        );
        painter.line_segment(
            [
                handle_center + Vec2::new(arrow_offset, 0.0),
                handle_center + Vec2::new(arrow_offset - 3.0, -3.0),
            ],
            Stroke::new(2.0, Color32::WHITE),
        );

        // Handle separator dragging
        let separator_hit_rect = Rect::from_center_size(
            Pos2::new(separator_x, canvas.center().y),
            Vec2::new(50.0, canvas.height()),
        );

        let pointer_pos = ui.input(|i| i.pointer.hover_pos());
        let pointer_in_separator = pointer_pos
            .map(|p| separator_hit_rect.contains(p))
            .unwrap_or(false);

        if pointer_in_separator || self.dragging_separator {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        }

        if response.drag_started() {
            if let Some(pos) = pointer_pos {
                if separator_hit_rect.contains(pos) {
                    self.dragging_separator = true;
                } else {
                    self.panning = true;
                    self.last_pan_pos = Some(pos);
                }
            }
        }

        if response.dragged() {
            if self.dragging_separator {
                if let Some(pos) = pointer_pos {
                    self.separator = ((pos.x - canvas.left()) / canvas.width()).clamp(0.01, 0.99);
                }
            } else if self.panning {
                if let Some(pos) = pointer_pos {
                    if let Some(last) = self.last_pan_pos {
                        self.pan_offset += pos - last;
                    }
                    self.last_pan_pos = Some(pos);
                }
            }
        }

        if response.drag_stopped() {
            self.dragging_separator = false;
            self.panning = false;
            self.last_pan_pos = None;
        }

        // Handle zoom with scroll wheel
        let scroll_delta = ui.input(|i| i.smooth_scroll_delta.y);
        if scroll_delta != 0.0 && response.hovered() {
            let zoom_factor = 1.0 + scroll_delta * 0.002;
            let new_zoom = (self.zoom * zoom_factor).clamp(0.1, 50.0);

            if let Some(cursor) = pointer_pos {
                let cursor_rel = cursor - (canvas.center() + self.pan_offset);
                self.pan_offset -= cursor_rel * (new_zoom / self.zoom - 1.0);
            }

            self.zoom = new_zoom;
        }

        // Also handle pinch zoom
        let multi_zoom = ui.input(|i| i.zoom_delta());
        if multi_zoom != 1.0 && response.hovered() {
            let new_zoom = (self.zoom * multi_zoom).clamp(0.1, 50.0);
            if let Some(cursor) = pointer_pos {
                let cursor_rel = cursor - (canvas.center() + self.pan_offset);
                self.pan_offset -= cursor_rel * (new_zoom / self.zoom - 1.0);
            }
            self.zoom = new_zoom;
        }
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
                painter.line_segment([prev, p], Stroke::new(3.0, Color32::from_rgb(100, 160, 255)));
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

        // Draw dashed border
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
            "First image (Drag and drop or click to browse)"
        } else {
            "Second image (Drag and drop or click to browse)"
        };

        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            text,
            egui::FontId::proportional(20.0),
            Color32::from_gray(180),
        );

        // Click to open file picker
        if response.interact(Sense::click()).clicked() {
            let side = if self.left.is_none() {
                crate::app::Side::Left
            } else {
                crate::app::Side::Right
            };
            self.request_open(side);
        }
    }
}
