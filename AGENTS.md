# Image Compare — Agent Guide

A WebAssembly app (Rust + [eframe/egui](https://github.com/emilk/egui)) for comparing two
images side-by-side with a draggable separator, pan/zoom, and Sobel edge detection. It runs
**only in the browser** — there is no native binary.

## Build & Run

- **Dev server:** `trunk serve` — serves at `http://127.0.0.1:8080` (see [Trunk.toml](Trunk.toml)). A `trunk` terminal is usually already running; hot-reloads on save.
- **Release build:** `trunk build --release` — outputs to `dist/`.
- **Type-check fast:** `cargo check --target wasm32-unknown-unknown` (the only valid target; the app uses `web-sys`/`wasm-bindgen` and will not compile for the host).
- Edition is **2024**, toolchain `rustc 1.96+`.
- `dist/` is generated build output — never edit files there.

## Architecture

Single-page app. The entry point [src/main.rs](src/main.rs) boots `eframe::WebRunner` on the
`#the_canvas_id` canvas from [index.html](index.html). Logic is split across:

| Module | Responsibility |
|--------|----------------|
| [src/app.rs](src/app.rs) | `App` shell, `update()` loop, side accessors, `set_edges_enabled` / `clear_all`. |
| [src/state.rs](src/state.rs) | `ViewState`, `EdgeState`/`EdgeCache`, `AsyncIo`, `DropState`, image id counter. |
| [src/load.rs](src/load.rs) | Drop routing, file picker, paste listener, channel polling, texture build. |
| [src/drawing.rs](src/drawing.rs) | egui rendering & input: comparison canvas, headers, loading + drop-zone views. |
| [src/decode.rs](src/decode.rs) | `decode_via_browser()` — async decode via `createImageBitmap` + canvas `getImageData`. |
| [src/edge.rs](src/edge.rs) | `sobel_edge_detect()` + cached edge texture upload. |
| [src/types.rs](src/types.rs) | Shared contracts: `Side`, `LoadedFile`, `DecodedImage`, `DecodeOutcome`, `LoadedImage`. |

Images are stored as `App.images: [Option<LoadedImage>; 2]`, indexed by `Side` (`Left = 0`,
`Right = 1`). Edge caches use the same layout (`EdgeState.caches`).

### Data flow (image load)

```
drop / file-picker / paste → LoadedFile (file_tx) → poll_picked_files()
  → load_and_set() → decode_via_browser() [async, off-thread]
  → DecodeOutcome (decoded_tx) → poll_decoded_images()
  → build_loaded_image() (uploads texture) → set_image() → draw_comparison()
```

Paste is handled by a one-time document `paste` listener (not egui key
handling). The first `image/*` clipboard item is auto-routed like a single-file
drop (first empty side, else right) via `App::auto_route_side()`.

## Project-specific conventions

- **No threads.** Everything runs on the browser event loop. Async tasks are spawned with
  `wasm_bindgen_futures::spawn_local` and deliver results over `std::sync::mpsc` channels that
  the `update()` loop drains every frame. Always `ctx.request_repaint()` after sending a result
  so the UI wakes up.
- **Decode in the browser, not in Rust.** Image formats (PNG/JPEG/WebP/AVIF/BMP/TIFF) are
  decoded via `createImageBitmap` because the browser's decoder is 10–50× faster than a WASM
  one. Don't add a Rust image-decoding crate.
- **Keep RGBA in memory.** `LoadedImage.rgba` caches decoded pixels so edge detection can rerun
  without re-decoding.
- **Cheap cache keys.** Each `LoadedImage` gets a monotonic `id: u64` used as the edge-detection
  cache key (`EdgeCache.key`) — compare ids, never hash pixels.
- **web-sys features are explicit.** Any new browser API (DOM, canvas, blob) must be added to the
  `web-sys` `features` list in [Cargo.toml](Cargo.toml) or it won't link.
- **Dev builds optimize dependencies.** [Cargo.toml](Cargo.toml) sets `opt-level = 3` for deps
  and `1` for our crate so per-pixel loops aren't unbearably slow in `trunk serve`. Don't remove
  these profile overrides.
- **Decode failures are silent.** All errors funnel into `DecodeOutcome::Failed`, which only
  clears the loading state — there is no error UI by design.
- **Responsive layout.** Below `NARROW_BREAKPOINT` (640px, in [src/types.rs](src/types.rs)) the
  header switches from `draw_header_wide` to `draw_header_narrow`.

## Gotchas

- Logs/panics go to the **browser console** (via `eframe::WebLogger`), not a terminal. Use
  `log::*` macros; `println!` won't appear.
- `cargo run`/`cargo build` for the host target will fail — always target `wasm32-unknown-unknown`
  or use `trunk`.
- Event closures for file pickers are kept alive with `.forget()` — intentional, not a leak bug.
