use eframe::egui;
use egui::{ColorImage, TextureOptions};

use crate::app::App;

/// Sobel edge detection over raw RGBA8 pixels, run synchronously.
pub fn sobel_edge_detect(rgba: &[u8], w: usize, h: usize) -> ColorImage {
    let mut output = vec![0u8; w * h * 4];

    let luminance = |px: usize, py: usize| -> f32 {
        let i = (py * w + px) * 4;
        0.299 * rgba[i] as f32 + 0.587 * rgba[i + 1] as f32 + 0.114 * rgba[i + 2] as f32
    };

    output.chunks_mut(w * 4).enumerate().for_each(|(y, row)| {
        if y == 0 || y >= h - 1 {
            return;
        }
        for x in 1..w - 1 {
            let gx = -luminance(x - 1, y - 1) - 2.0 * luminance(x - 1, y) - luminance(x - 1, y + 1)
                + luminance(x + 1, y - 1)
                + 2.0 * luminance(x + 1, y)
                + luminance(x + 1, y + 1);

            let gy = -luminance(x - 1, y - 1) - 2.0 * luminance(x, y - 1) - luminance(x + 1, y - 1)
                + luminance(x - 1, y + 1)
                + 2.0 * luminance(x, y + 1)
                + luminance(x + 1, y + 1);

            let mag = (gx * gx + gy * gy).sqrt().min(255.0) as u8;
            let idx = x * 4;
            row[idx] = mag;
            row[idx + 1] = mag;
            row[idx + 2] = mag;
            row[idx + 3] = 255;
        }
    });

    ColorImage::from_rgba_unmultiplied([w, h], &output)
}

impl App {
    pub fn start_edge_compute(&mut self, ctx: &egui::Context) {
        if let Some(left) = &self.left {
            // Skip recomputation if the cached result matches the current input.
            if self.left_edge.is_none() || self.left_edge_key != Some(left.hash) {
                let edge_image = sobel_edge_detect(&left.rgba, left.size[0], left.size[1]);
                self.left_edge = Some(ctx.load_texture(
                    format!("{}_edge", left.name),
                    edge_image,
                    TextureOptions::LINEAR,
                ));
                self.left_edge_key = Some(left.hash);
            }
        } else {
            self.left_edge = None;
            self.left_edge_key = None;
        }

        if let Some(right) = &self.right {
            if self.right_edge.is_none() || self.right_edge_key != Some(right.hash) {
                let edge_image = sobel_edge_detect(&right.rgba, right.size[0], right.size[1]);
                self.right_edge = Some(ctx.load_texture(
                    format!("{}_edge", right.name),
                    edge_image,
                    TextureOptions::LINEAR,
                ));
                self.right_edge_key = Some(right.hash);
            }
        } else {
            self.right_edge = None;
            self.right_edge_key = None;
        }
    }
}
