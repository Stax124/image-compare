use egui::ColorImage;
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;

use crate::app::App;
use crate::edge::sobel_edge_detect;
use crate::state::EdgePolarity;
use crate::types::{LoadedImage, Side};

/// Lossy encode quality for JPG / WebP / AVIF (`canvas.toBlob` quality argument).
///
/// 0.92 is near-archive quality and bloats high-frequency content (e.g. Sobel
/// edge maps). ~0.85 is visually fine for comparison exports and much smaller.
const LOSSY_QUALITY: f64 = 0.85;

/// Download formats offered by the Export menu.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    Png,
    Jpg,
    Webp,
    Avif,
}

impl ExportFormat {
    pub const ALL: [ExportFormat; 4] = [
        ExportFormat::Png,
        ExportFormat::Jpg,
        ExportFormat::Webp,
        ExportFormat::Avif,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ExportFormat::Png => "PNG",
            ExportFormat::Jpg => "JPG",
            ExportFormat::Webp => "WebP",
            ExportFormat::Avif => "AVIF",
        }
    }

    pub fn mime(self) -> &'static str {
        match self {
            ExportFormat::Png => "image/png",
            ExportFormat::Jpg => "image/jpeg",
            ExportFormat::Webp => "image/webp",
            ExportFormat::Avif => "image/avif",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            ExportFormat::Png => "png",
            ExportFormat::Jpg => "jpg",
            ExportFormat::Webp => "webp",
            ExportFormat::Avif => "avif",
        }
    }

    /// Whether `toBlob` should be passed a quality argument.
    pub fn is_lossy(self) -> bool {
        !matches!(self, ExportFormat::Png)
    }
}

/// File-name stem from a path-like name: strip directories and the last extension.
fn file_stem(name: &str) -> &str {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    match base.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() && !ext.is_empty() && ext.len() <= 5 => stem,
        _ => base,
    }
}

/// Keep only filesystem-friendly characters for a download filename segment.
fn sanitize_stem(stem: &str) -> String {
    let mut out: String = stem
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    // Collapse runs of '-' and trim.
    while out.contains("--") {
        out = out.replace("--", "-");
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() {
        "image".into()
    } else {
        // Cap length so the full download name stays reasonable.
        out.chars().take(40).collect()
    }
}

fn export_filename(left: &LoadedImage, right: &LoadedImage, format: ExportFormat) -> String {
    let left_stem = sanitize_stem(file_stem(&left.name));
    let right_stem = sanitize_stem(file_stem(&right.name));
    format!(
        "compare-{left_stem}-vs-{right_stem}.{}",
        format.extension()
    )
}

/// Convert a `ColorImage` (edge output) back to tightly packed RGBA8.
fn color_image_to_rgba(img: &ColorImage) -> Vec<u8> {
    let mut out = Vec::with_capacity(img.pixels.len() * 4);
    for px in &img.pixels {
        out.extend_from_slice(&px.to_array());
    }
    out
}

/// Source buffer used on one side of the composite (original or Sobel).
struct SideBuffer<'a> {
    rgba: &'a [u8],
    w: usize,
    h: usize,
}

/// Contain-fit of an image into `out_w`×`out_h` with scale ≤ 1, centered.
struct FitPlacement {
    /// Top-left of the drawn image in output coordinates.
    ox: f32,
    oy: f32,
    /// Displayed size in output pixels.
    dw: f32,
    dh: f32,
    src_w: usize,
    src_h: usize,
}

impl FitPlacement {
    fn new(src_w: usize, src_h: usize, out_w: usize, out_h: usize) -> Self {
        let scale = (out_w as f32 / src_w as f32)
            .min(out_h as f32 / src_h as f32)
            .min(1.0);
        let dw = src_w as f32 * scale;
        let dh = src_h as f32 * scale;
        let ox = (out_w as f32 - dw) * 0.5;
        let oy = (out_h as f32 - dh) * 0.5;
        Self {
            ox,
            oy,
            dw,
            dh,
            src_w,
            src_h,
        }
    }

    /// Nearest-neighbor sample; opaque black outside the fitted image.
    fn sample(&self, rgba: &[u8], x: usize, y: usize) -> [u8; 4] {
        let fx = x as f32 + 0.5;
        let fy = y as f32 + 0.5;
        if fx < self.ox || fy < self.oy || fx >= self.ox + self.dw || fy >= self.oy + self.dh {
            return [0, 0, 0, 255];
        }
        let u = (fx - self.ox) / self.dw;
        let v = (fy - self.oy) / self.dh;
        let sx = ((u * self.src_w as f32) as usize).min(self.src_w - 1);
        let sy = ((v * self.src_h as f32) as usize).min(self.src_h - 1);
        let i = (sy * self.src_w + sx) * 4;
        [rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]]
    }
}

/// Build the full-resolution comparison composite (no separator chrome).
///
/// Output size is `max(w)×max(h)`. Each side is contain-fitted (never upscaled)
/// and centered; pixels left of `separator * width` come from the left buffer,
/// the rest from the right. Letterbox is opaque black.
fn composite_comparison(
    left: SideBuffer<'_>,
    right: SideBuffer<'_>,
    separator: f32,
) -> (Vec<u8>, [usize; 2]) {
    let out_w = left.w.max(right.w).max(1);
    let out_h = left.h.max(right.h).max(1);
    let left_fit = FitPlacement::new(left.w, left.h, out_w, out_h);
    let right_fit = FitPlacement::new(right.w, right.h, out_w, out_h);

    let split_x = ((separator.clamp(0.0, 1.0) * out_w as f32) as usize).min(out_w);
    let mut out = vec![0u8; out_w * out_h * 4];

    for y in 0..out_h {
        for x in 0..out_w {
            let px = if x < split_x {
                left_fit.sample(left.rgba, x, y)
            } else {
                right_fit.sample(right.rgba, x, y)
            };
            let i = (y * out_w + x) * 4;
            out[i] = px[0];
            out[i + 1] = px[1];
            out[i + 2] = px[2];
            out[i + 3] = px[3];
        }
    }

    (out, [out_w, out_h])
}

/// Encode RGBA8 via an offscreen canvas and trigger a browser download.
fn download_rgba(rgba: Vec<u8>, size: [usize; 2], format: ExportFormat, filename: String) {
    let w = size[0] as u32;
    let h = size[1] as u32;
    if w == 0 || h == 0 {
        log::warn!("export: empty image");
        return;
    }

    let Some(window) = web_sys::window() else {
        log::warn!("export: no window");
        return;
    };
    let Some(document) = window.document() else {
        log::warn!("export: no document");
        return;
    };

    let Ok(canvas) = document.create_element("canvas") else {
        log::warn!("export: cannot create canvas");
        return;
    };
    let Ok(canvas) = canvas.dyn_into::<web_sys::HtmlCanvasElement>() else {
        return;
    };
    canvas.set_width(w);
    canvas.set_height(h);

    let Ok(Some(ctx_obj)) = canvas.get_context("2d") else {
        log::warn!("export: no 2d context");
        return;
    };
    let Ok(ctx) = ctx_obj.dyn_into::<web_sys::CanvasRenderingContext2d>() else {
        return;
    };

    let image_data = match web_sys::ImageData::new_with_u8_clamped_array_and_sh(
        wasm_bindgen::Clamped(&rgba),
        w,
        h,
    ) {
        Ok(data) => data,
        Err(err) => {
            log::warn!("export: ImageData failed: {err:?}");
            return;
        }
    };

    if ctx.put_image_data(&image_data, 0.0, 0.0).is_err() {
        log::warn!("export: putImageData failed");
        return;
    }

    let mime = format.mime();
    // `toBlob` may invoke the callback with `null` when the type is unsupported
    // (common for AVIF in some browsers). When the type is unsupported the
    // browser often falls back to PNG — we must not save that as `.avif`/etc.
    let callback = Closure::once(Box::new(move |blob: wasm_bindgen::JsValue| {
        if blob.is_null() || blob.is_undefined() {
            log::warn!("export: toBlob returned null (format may be unsupported)");
            return;
        }
        match blob.dyn_into::<web_sys::Blob>() {
            Ok(blob) => finalize_export_blob(blob, format, filename),
            Err(_) => log::warn!("export: toBlob callback value is not a Blob"),
        }
    }) as Box<dyn FnOnce(wasm_bindgen::JsValue)>);

    let result = if format.is_lossy() {
        canvas.to_blob_with_type_and_encoder_options(
            callback.as_ref().unchecked_ref(),
            mime,
            &wasm_bindgen::JsValue::from_f64(LOSSY_QUALITY),
        )
    } else {
        canvas.to_blob_with_type(callback.as_ref().unchecked_ref(), mime)
    };

    match result {
        Ok(()) => {
            // Keep the closure alive until the browser invokes it.
            callback.forget();
        }
        Err(err) => {
            log::warn!("export: toBlob({mime}) failed: {err:?}");
        }
    }
}

/// Check that the browser actually produced the requested type, then download.
///
/// Per the HTML spec, unsupported `type` values make `toBlob` fall back to
/// `image/png`. Without this check, an "AVIF" export can be a multi‑MB PNG
/// saved with a `.avif` extension.
fn finalize_export_blob(blob: web_sys::Blob, format: ExportFormat, filename: String) {
    let expected = format.mime();
    let actual = blob.type_();
    // Empty type is rare; treat as accept only when we asked for PNG (some
    // engines omit the MIME on PNG blobs).
    let type_ok = if actual.is_empty() {
        matches!(format, ExportFormat::Png)
    } else {
        actual.eq_ignore_ascii_case(expected)
    };

    if !type_ok {
        log::warn!(
            "export: requested {expected} but browser produced '{actual}' \
             (format not supported for canvas encoding). Download cancelled — \
             try PNG, JPG, or WebP instead."
        );
        return;
    }

    let size = blob.size();
    log::info!(
        "export: {filename} ({actual}, {:.1} KB)",
        size / 1024.0
    );

    trigger_blob_download(&blob, &filename);
}

fn trigger_blob_download(blob: &web_sys::Blob, filename: &str) {
    let Ok(url) = web_sys::Url::create_object_url_with_blob(blob) else {
        log::warn!("export: createObjectURL failed");
        return;
    };

    let Some(window) = web_sys::window() else {
        let _ = web_sys::Url::revoke_object_url(&url);
        return;
    };
    let Some(document) = window.document() else {
        let _ = web_sys::Url::revoke_object_url(&url);
        return;
    };

    let Ok(anchor) = document.create_element("a") else {
        let _ = web_sys::Url::revoke_object_url(&url);
        return;
    };
    let Ok(anchor) = anchor.dyn_into::<web_sys::HtmlAnchorElement>() else {
        let _ = web_sys::Url::revoke_object_url(&url);
        return;
    };

    anchor.set_href(&url);
    anchor.set_download(filename);
    // Required in some browsers for the click to be treated as a user gesture chain.
    anchor.style().set_property("display", "none").ok();
    if let Some(body) = document.body() {
        let _ = body.append_child(&anchor);
        anchor.click();
        let _ = body.remove_child(&anchor);
    } else {
        anchor.click();
    }

    let _ = web_sys::Url::revoke_object_url(&url);
}

impl App {
    /// Composite the current comparison (respecting edge mode + separator) and
    /// download it in `format`. No-op if either side is empty.
    pub(crate) fn export_comparison(&self, format: ExportFormat) {
        let Some(left) = self.image(Side::Left) else {
            return;
        };
        let Some(right) = self.image(Side::Right) else {
            return;
        };

        // Edge textures are GPU-only; recompute Sobel into RGBA for the export.
        let (left_owned, right_owned): (Option<Vec<u8>>, Option<Vec<u8>>) = if self.edges.enabled {
            let polarity = self.edges.polarity;
            (
                Some(edge_rgba(left, polarity)),
                Some(edge_rgba(right, polarity)),
            )
        } else {
            (None, None)
        };

        let left_buf = SideBuffer {
            rgba: left_owned.as_deref().unwrap_or(&left.rgba),
            w: left.size[0],
            h: left.size[1],
        };
        let right_buf = SideBuffer {
            rgba: right_owned.as_deref().unwrap_or(&right.rgba),
            w: right.size[0],
            h: right.size[1],
        };

        let filename = export_filename(left, right, format);
        let separator = self.view.separator;
        let (rgba, size) = composite_comparison(left_buf, right_buf, separator);

        // Encoding + download are async (toBlob callback); composite was sync.
        download_rgba(rgba, size, format, filename);
    }
}

fn edge_rgba(img: &LoadedImage, polarity: EdgePolarity) -> Vec<u8> {
    let edge = sobel_edge_detect(&img.rgba, img.size[0], img.size[1], polarity);
    color_image_to_rgba(&edge)
}
