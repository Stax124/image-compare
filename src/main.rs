mod app;
mod drawing;
mod edge;

use app::App;

fn main() -> eframe::Result {
    env_logger::init();

    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Image Compare")
            .with_inner_size([1280.0, 720.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };

    eframe::run_native(
        "Image Compare",
        options,
        Box::new(|_cc| Ok(Box::new(App::default()))),
    )
}
