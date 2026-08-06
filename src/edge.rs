use eframe::egui;
use egui::{ColorImage, TextureOptions};

use crate::app::App;
use crate::types::Side;

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
    pub(crate) fn start_edge_compute(&mut self, ctx: &egui::Context) {
        for side in Side::ALL {
            let Some(img) = self.image(side) else {
                self.edges.clear_side(side);
                continue;
            };
            if !self.edges.cache(side).needs_recompute(img.id) {
                continue;
            }

            // Copy what we need so we can mutably borrow `edges` after.
            let id = img.id;
            let name = img.name.clone();
            let size = img.size;
            let edge_image = sobel_edge_detect(&img.rgba, size[0], size[1]);
            let texture =
                ctx.load_texture(format!("{name}_edge"), edge_image, TextureOptions::LINEAR);

            let cache = self.edges.cache_mut(side);
            cache.texture = Some(texture);
            cache.key = Some(id);
        }
    }
}
