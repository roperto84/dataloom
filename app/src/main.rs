mod app;
mod db;
mod import;
mod menu;
mod settings;
mod sqlfmt;
mod theme;
mod tunnel;
mod views;
mod widgets;
mod xlsx;

use std::path::PathBuf;

#[derive(Default)]
struct Args {
    screenshot: Option<PathBuf>,
}

fn parse_args() -> Args {
    let mut args = Args::default();
    let mut iter = std::env::args().skip(1);
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--screenshot" => {
                if let Some(path) = iter.next() {
                    args.screenshot = Some(PathBuf::from(path));
                }
            }
            other => {
                if let Some(rest) = other.strip_prefix("--screenshot=") {
                    args.screenshot = Some(PathBuf::from(rest));
                }
            }
        }
    }
    args
}

fn app_icon() -> Option<eframe::egui::IconData> {
    let bytes = include_bytes!("../assets/icon.png");
    let image = image::load_from_memory(bytes).ok()?.to_rgba8();
    let (width, height) = (image.width(), image.height());
    Some(eframe::egui::IconData {
        rgba: image.into_raw(),
        width,
        height,
    })
}

fn main() -> eframe::Result<()> {
    let args = parse_args();

    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_inner_size([1400.0, 880.0])
        .with_min_inner_size([980.0, 620.0])
        .with_title("Dataloom");
    if let Some(icon) = app_icon() {
        viewport = viewport.with_icon(icon);
    }

    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    eframe::run_native(
        "Dataloom",
        options,
        Box::new(move |cc| Ok(Box::new(app::DataloomApp::new(cc, args.screenshot.clone())))),
    )
}
