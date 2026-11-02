use eframe::egui;
use std::sync::mpsc;
use wasm_bindgen::JsCast;

use crate::types::{DecodeOutcome, DecodedImage, Side};

/// Decode an encoded image using the browser's native decoder, then deliver the
/// raw RGBA8 pixels over `tx`. Handles every format the browser supports
/// (PNG, JPEG, WebP, AVIF, …), falling back to jxl-rs for JPEG XL.
///
/// Bytes are wrapped in a `Blob` and handed to `createImageBitmap`, which
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
    wasm_bindgen_futures::spawn_local(async move {
        let file_size = bytes.len();
        let finish = |outcome: DecodeOutcome| {
            if tx.send(outcome).is_ok() {
                ctx.request_repaint();
            }
        };

        let decoded = match decode_blob(&bytes).await {
            Some(decoded) => Some(decoded),
            None => decode_jxl(&bytes),
        };
        let Some((size, rgba)) = decoded else {
            finish(DecodeOutcome::Failed);
            return;
        };
        finish(DecodeOutcome::Ok(DecodedImage {
            side,
            name,
            file_size,
            size,
            rgba,
        }));
    });
}

async fn decode_blob(bytes: &[u8]) -> Option<([usize; 2], Vec<u8>)> {
    let array = js_sys::Uint8Array::from(bytes);
    let parts = js_sys::Array::new();
    parts.push(&array);
    let blob = web_sys::Blob::new_with_u8_array_sequence(&parts).ok()?;

    // `createImageBitmap` performs the decode, possibly off the main thread.
    let window = web_sys::window()?;
    let promise = window.create_image_bitmap_with_blob(&blob).ok()?;
    let bitmap: web_sys::ImageBitmap = wasm_bindgen_futures::JsFuture::from(promise)
        .await
        .ok()?
        .dyn_into()
        .ok()?;

    let width = bitmap.width();
    let height = bitmap.height();
    if width == 0 || height == 0 {
        bitmap.close();
        return None;
    }

    let document = window.document()?;
    let canvas: web_sys::HtmlCanvasElement =
        document.create_element("canvas").ok()?.dyn_into().ok()?;
    canvas.set_width(width);
    canvas.set_height(height);
    let context: web_sys::CanvasRenderingContext2d =
        canvas.get_context("2d").ok()??.dyn_into().ok()?;
    let draw_result = context.draw_image_with_image_bitmap(&bitmap, 0.0, 0.0);
    // Release the bitmap as soon as it has been drawn.
    bitmap.close();
    draw_result.ok()?;

    let image_data = context
        .get_image_data(0.0, 0.0, width as f64, height as f64)
        .ok()?;

    Some(([width as usize, height as usize], image_data.data().0))
}

/// Synchronous, so it blocks the UI while decoding.
fn decode_jxl(mut input: &[u8]) -> Option<([usize; 2], Vec<u8>)> {
    use jxl::api::{
        AlphaMode, Event, JxlDecoder, JxlDecoderOptions, JxlOutputBuffer, JxlPixelFormat,
    };

    let mut options = JxlDecoderOptions::default();
    options.alpha_mode = AlphaMode::Unpremultiplied;
    // Rejects oversized headers before the RGBA buffer below is allocated, keeping it under 4 GiB.
    options.sample_limit = Some(1 << 28);
    let mut decoder = JxlDecoder::new(options);
    if decoder.process(&mut input, None, None).ok()? != Event::BasicInfo {
        return None;
    }
    let basic_info = decoder.basic_info()?;
    let (width, height) = basic_info.size;
    let extra_channel_count = basic_info.extra_channels.len();
    decoder
        .set_pixel_format(JxlPixelFormat::rgba8(extra_channel_count))
        .ok()?;

    let mut rgba = vec![0u8; width * height * 4];
    loop {
        let mut buffers = [JxlOutputBuffer::new(&mut rgba, height, width * 4)];
        match decoder.process(&mut input, Some(&mut buffers), None).ok()? {
            Event::FrameComplete { .. } => return Some(([width, height], rgba)),
            Event::NeedMoreInput { .. } | Event::Complete => return None,
            Event::BasicInfo | Event::FrameHeader => {}
        }
    }
}
