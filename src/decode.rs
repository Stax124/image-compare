use eframe::egui;
use std::sync::mpsc;

use crate::types::{DecodeOutcome, DecodedImage, Side};

/// Current high-resolution timestamp in milliseconds, used for timing logs.
/// Falls back to 0.0 if the Performance API is unavailable.
pub(crate) fn now_ms() -> f64 {
    web_sys::window()
        .and_then(|w| w.performance())
        .map(|p| p.now())
        .unwrap_or(0.0)
}

/// Decode an encoded image using the browser's native decoder, then deliver the
/// raw RGBA8 pixels over `tx`. This handles every format the browser supports,
/// including PNG, JPEG, WebP, and AVIF.
///
/// The bytes are wrapped in a `Blob` and handed to `createImageBitmap`, which
/// decodes the image (potentially off the main thread). The resulting
/// `ImageBitmap` is drawn onto an offscreen canvas and read back via
/// `getImageData`.
pub fn decode_via_browser(
    ctx: egui::Context,
    tx: mpsc::Sender<DecodeOutcome>,
    side: Side,
    name: String,
    bytes: Vec<u8>,
) {
    use wasm_bindgen::JsCast;

    wasm_bindgen_futures::spawn_local(async move {
        let file_size = bytes.len();
        let t_start = now_ms();

        // Wrap the encoded bytes in a Blob.
        let array = js_sys::Uint8Array::from(bytes.as_slice());
        let parts = js_sys::Array::new();
        parts.push(&array);
        let Ok(blob) = web_sys::Blob::new_with_u8_array_sequence(&parts) else {
            let _ = tx.send(DecodeOutcome::Failed);
            ctx.request_repaint();
            return;
        };

        let log_name = name.clone();
        let decoded = async move {
            // `createImageBitmap` performs the decode, possibly off the main
            // thread, which is faster than going through an `HtmlImageElement`.
            let window = web_sys::window()?;
            let promise = window.create_image_bitmap_with_blob(&blob).ok()?;
            let bitmap: web_sys::ImageBitmap = wasm_bindgen_futures::JsFuture::from(promise)
                .await
                .ok()?
                .dyn_into()
                .ok()?;
            let t_decode = now_ms();

            let width = bitmap.width();
            let height = bitmap.height();
            if width == 0 || height == 0 {
                bitmap.close();
                return None;
            }

            // Draw onto an offscreen canvas and read the pixels back.
            let document = window.document()?;
            let canvas: web_sys::HtmlCanvasElement =
                document.create_element("canvas").ok()?.dyn_into().ok()?;
            canvas.set_width(width);
            canvas.set_height(height);
            let context: web_sys::CanvasRenderingContext2d =
                canvas.get_context("2d").ok()??.dyn_into().ok()?;
            let draw_result = context.draw_image_with_image_bitmap(&bitmap, 0.0, 0.0);
            // Release the bitmap's resources as soon as it has been drawn.
            bitmap.close();
            draw_result.ok()?;
            let t_draw = now_ms();
            let image_data = context
                .get_image_data(0.0, 0.0, width as f64, height as f64)
                .ok()?;
            let rgba = image_data.data().0;
            let t_readback = now_ms();

            log::info!(
                "[timing] decode '{}' ({}x{}, {} bytes): createImageBitmap {:.1}ms, draw_image {:.1}ms, get_image_data {:.1}ms, decode total {:.1}ms",
                log_name,
                width,
                height,
                file_size,
                t_decode - t_start,
                t_draw - t_decode,
                t_readback - t_draw,
                t_readback - t_start,
            );

            Some(DecodedImage {
                side,
                name,
                file_size,
                size: [width as usize, height as usize],
                rgba,
            })
        }
        .await;

        let outcome = match decoded {
            Some(decoded) => DecodeOutcome::Ok(decoded),
            None => DecodeOutcome::Failed,
        };
        if tx.send(outcome).is_ok() {
            // Wake the UI so the freshly decoded image (or cleared loading
            // state) is picked up.
            ctx.request_repaint();
        }
    });
}
