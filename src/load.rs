use eframe::egui::{self, TextureOptions};
use std::sync::mpsc;
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;

use crate::app::App;
use crate::decode::decode_via_browser;
use crate::state::next_image_id;
use crate::types::{DecodeOutcome, LoadedFile, LoadedImage, Side};

/// Read a browser `File` into a byte vector.
async fn file_to_bytes(file: &web_sys::File) -> Option<Vec<u8>> {
    let buffer = wasm_bindgen_futures::JsFuture::from(file.array_buffer())
        .await
        .ok()?;
    Some(js_sys::Uint8Array::new(&buffer).to_vec())
}

/// Deliver a loaded file on the channel and wake the UI.
fn send_loaded_file(tx: &mpsc::Sender<LoadedFile>, ctx: &egui::Context, file: LoadedFile) {
    if tx.send(file).is_ok() {
        // Wake the UI so the file is polled even when no other input drives frames.
        ctx.request_repaint();
    }
}

/// Filename for a clipboard image; synthesizes one when the OS omits it.
fn clipboard_filename(file: &web_sys::File, mime: &str) -> String {
    let name = file.name();
    if !name.is_empty() {
        return name;
    }
    let ext = mime.rsplit('/').next().unwrap_or("png");
    let ext = match ext {
        "jpeg" => "jpg",
        "svg+xml" => "svg",
        other => other,
    };
    format!("clipboard.{ext}")
}

/// Multi-file drop: first file → left, second → right; ignore extras.
/// Single-file drop uses auto-route instead (caller handles that).
fn multi_drop_side(index: usize) -> Option<Side> {
    match index {
        0 => Some(Side::Left),
        1 => Some(Side::Right),
        _ => None,
    }
}

impl App {
    /// Build a `LoadedImage` (texture + cached pixels) from decoded RGBA8 data.
    fn build_loaded_image(
        &self,
        ctx: &egui::Context,
        name: &str,
        file_size: usize,
        size: [usize; 2],
        raw: Vec<u8>,
    ) -> LoadedImage {
        let color_image = egui::ColorImage::from_rgba_unmultiplied(size, &raw);
        let texture = ctx.load_texture(name, color_image, TextureOptions::LINEAR);

        LoadedImage {
            texture,
            name: name.to_string(),
            size,
            file_size,
            rgba: raw,
            id: next_image_id(),
        }
    }

    /// Decode `bytes` asynchronously and assign the result to `side`.
    fn load_and_set(&mut self, ctx: &egui::Context, side: Side, name: &str, bytes: &[u8]) {
        self.io.pending_loads += 1;
        decode_via_browser(
            ctx.clone(),
            self.io.decoded_tx.clone(),
            side,
            name.to_string(),
            bytes.to_vec(),
        );
    }

    pub(crate) fn handle_dropped_files(&mut self, ctx: &egui::Context) {
        // On the web each dropped file is read by its own async future and
        // pushed into `dropped_files` in a separate frame, so a multi-file drop
        // never arrives all at once. Track drag-hover to learn how many files
        // belong to the current drop and route them as bytes trickle in.
        let hovering_count = ctx.input(|i| i.raw.hovered_files.len());
        if hovering_count > 0 {
            if !self.drop.was_hovering {
                self.drop.index = 0;
            }
            self.drop.expected = hovering_count;
        }
        self.drop.was_hovering = hovering_count > 0;

        let dropped_files: Vec<(String, Vec<u8>)> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.bytes.as_ref().map(|b| (f.name.clone(), b.to_vec())))
                .collect()
        });

        for (name, bytes) in dropped_files {
            let side = if self.drop.expected >= 2 {
                match multi_drop_side(self.drop.index) {
                    Some(side) => side,
                    None => {
                        self.drop.index += 1;
                        continue;
                    }
                }
            } else {
                self.auto_route_side()
            };
            self.drop.index += 1;
            self.load_and_set(ctx, side, &name, &bytes);
        }
    }

    /// Open the native browser file picker and deliver the picked file over a channel.
    pub(crate) fn request_open(&self, ctx: &egui::Context, side: Side) {
        let tx = self.io.file_tx.clone();
        let ctx = ctx.clone();
        let Some(window) = web_sys::window() else {
            return;
        };
        let Some(document) = window.document() else {
            return;
        };
        let Some(input) = document
            .create_element("input")
            .ok()
            .and_then(|el| el.dyn_into::<web_sys::HtmlInputElement>().ok())
        else {
            return;
        };
        input.set_type("file");
        input.set_accept(
            "image/png,image/jpeg,image/webp,image/bmp,image/tiff,image/avif,.png,.jpg,.jpeg,.webp,.bmp,.tiff,.avif",
        );

        let input_clone = input.clone();
        let closure = Closure::<dyn FnMut()>::new(move || {
            let Some(files) = input_clone.files() else {
                return;
            };
            let Some(file) = files.get(0) else {
                return;
            };
            let name = file.name();
            let tx = tx.clone();
            let ctx = ctx.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let Some(bytes) = file_to_bytes(&file).await else {
                    return;
                };
                send_loaded_file(
                    &tx,
                    &ctx,
                    LoadedFile {
                        side: Some(side),
                        name,
                        bytes,
                    },
                );
            });
        });
        input.set_onchange(Some(closure.as_ref().unchecked_ref()));
        // The closure must outlive this function so the change event can fire.
        closure.forget();

        input.click();
    }

    /// Install a one-shot document `paste` listener for clipboard images.
    ///
    /// Safe to call every frame: registration is guarded by a process-wide `Once`.
    pub(crate) fn install_paste_listener(&self, ctx: &egui::Context) {
        use std::sync::Once;

        static PASTE_LISTENER: Once = Once::new();
        let tx = self.io.file_tx.clone();
        let ctx = ctx.clone();

        PASTE_LISTENER.call_once(move || {
            let Some(window) = web_sys::window() else {
                return;
            };
            let Some(document) = window.document() else {
                return;
            };

            let closure =
                Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
                    let Ok(clipboard_event) = event.dyn_into::<web_sys::ClipboardEvent>() else {
                        return;
                    };
                    let Some(data) = clipboard_event.clipboard_data() else {
                        return;
                    };
                    let items = data.items();
                    for i in 0..items.length() {
                        let Some(item) = items.get(i) else {
                            continue;
                        };
                        let mime = item.type_();
                        if !mime.starts_with("image/") {
                            continue;
                        }
                        let Ok(Some(file)) = item.get_as_file() else {
                            continue;
                        };

                        clipboard_event.prevent_default();

                        let name = clipboard_filename(&file, &mime);
                        let tx = tx.clone();
                        let ctx = ctx.clone();
                        wasm_bindgen_futures::spawn_local(async move {
                            let Some(bytes) = file_to_bytes(&file).await else {
                                return;
                            };
                            send_loaded_file(
                                &tx,
                                &ctx,
                                LoadedFile {
                                    side: None,
                                    name,
                                    bytes,
                                },
                            );
                        });
                        // Only the first image in the clipboard is used.
                        break;
                    }
                });

            let _ = document
                .add_event_listener_with_callback("paste", closure.as_ref().unchecked_ref());
            // Keep the closure alive for the lifetime of the page.
            closure.forget();
        });
    }

    /// Drain files from the picker / paste channel and start decoding.
    pub(crate) fn poll_picked_files(&mut self, ctx: &egui::Context) {
        let pending: Vec<LoadedFile> = self.io.file_rx.try_iter().collect();
        for file in pending {
            let side = file.side.unwrap_or_else(|| self.auto_route_side());
            self.load_and_set(ctx, side, &file.name, &file.bytes);
        }
    }

    /// Drain browser-decoded images and upload them as textures.
    pub(crate) fn poll_decoded_images(&mut self, ctx: &egui::Context) {
        let pending: Vec<DecodeOutcome> = self.io.decoded_rx.try_iter().collect();
        for outcome in pending {
            self.io.pending_loads = self.io.pending_loads.saturating_sub(1);
            let DecodeOutcome::Ok(img) = outcome else {
                continue;
            };
            let loaded = self.build_loaded_image(ctx, &img.name, img.file_size, img.size, img.rgba);
            self.set_image(img.side, loaded);
            if self.edges.enabled {
                self.start_edge_compute(ctx);
            }
        }
    }
}
