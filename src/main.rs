mod app;
mod history;
mod pdf;

use std::env;
use std::path::PathBuf;
use std::process;

use winit::event_loop::EventLoop;

use app::App;

fn main() {
    let path = env::args().nth(1).map(PathBuf::from).unwrap_or_else(|| {
        eprintln!("usage: riftpdf <file.pdf>");
        process::exit(1);
    });

    if !path.exists() {
        eprintln!("file not found: {}", path.display());
        process::exit(1);
    }

    let app = match App::new(path) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("failed to open pdf: {e}");
            process::exit(1);
        }
    };

    let event_loop = EventLoop::new().expect("event loop");
    let mut app = app;
    event_loop.run_app(&mut app).expect("run");
}
