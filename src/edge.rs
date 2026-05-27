use eframe::egui;
use egui::{ColorImage, TextureOptions};
use rayon::prelude::*;
use std::sync::mpsc;

use crate::app::App;

pub enum EdgeResult {
    Left(ColorImage),
    Right(ColorImage),
}

pub fn sobel_edge_detect(img: &image::RgbaImage) -> ColorImage {
    let (w, h) = (img.width() as usize, img.height() as usize);
    let mut output = vec![0u8; w * h * 4];

    output
        .par_chunks_mut(w * 4)
        .enumerate()
        .for_each(|(y, row)| {
            if y == 0 || y >= h - 1 {
                return;
            }
            for x in 1..w - 1 {
                let luminance = |px: usize, py: usize| -> f32 {
                    let p = img.get_pixel(px as u32, py as u32);
                    0.299 * p[0] as f32 + 0.587 * p[1] as f32 + 0.114 * p[2] as f32
                };

                let gx =
                    -luminance(x - 1, y - 1) - 2.0 * luminance(x - 1, y) - luminance(x - 1, y + 1)
                        + luminance(x + 1, y - 1)
                        + 2.0 * luminance(x + 1, y)
                        + luminance(x + 1, y + 1);

                let gy =
                    -luminance(x - 1, y - 1) - 2.0 * luminance(x, y - 1) - luminance(x + 1, y - 1)
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
        let left_path = self.left.as_ref().map(|l| l.path.clone());
        let right_path = self.right.as_ref().map(|r| r.path.clone());

        let (tx, rx) = mpsc::channel();
        self.edge_rx = Some(rx);
        self.edge_computing = true;
        self.left_edge = None;
        self.right_edge = None;

        let repaint_ctx = ctx.clone();
        std::thread::spawn(move || {
            if let Some(path) = left_path {
                if let Ok(img) = image::open(&path) {
                    let rgba = img.to_rgba8();
                    let edge_image = sobel_edge_detect(&rgba);
                    let _ = tx.send(EdgeResult::Left(edge_image));
                }
            }
            if let Some(path) = right_path {
                if let Ok(img) = image::open(&path) {
                    let rgba = img.to_rgba8();
                    let edge_image = sobel_edge_detect(&rgba);
                    let _ = tx.send(EdgeResult::Right(edge_image));
                }
            }
            repaint_ctx.request_repaint();
        });
    }

    pub fn poll_edge_results(&mut self, ctx: &egui::Context) {
        if let Some(rx) = &self.edge_rx {
            let mut got_any = false;
            while let Ok(result) = rx.try_recv() {
                got_any = true;
                match result {
                    EdgeResult::Left(img) => {
                        let name = self
                            .left
                            .as_ref()
                            .map(|l| l.name.clone())
                            .unwrap_or_default();
                        self.left_edge = Some(ctx.load_texture(
                            format!("{}_edge", name),
                            img,
                            TextureOptions::LINEAR,
                        ));
                    }
                    EdgeResult::Right(img) => {
                        let name = self
                            .right
                            .as_ref()
                            .map(|r| r.name.clone())
                            .unwrap_or_default();
                        self.right_edge = Some(ctx.load_texture(
                            format!("{}_edge", name),
                            img,
                            TextureOptions::LINEAR,
                        ));
                    }
                }
            }
            let left_done = self.left.is_none() || self.left_edge.is_some();
            let right_done = self.right.is_none() || self.right_edge.is_some();
            if got_any && left_done && right_done {
                self.edge_computing = false;
                self.edge_rx = None;
            }
        }
    }
}
