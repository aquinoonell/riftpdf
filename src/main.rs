mod app;
mod history;
mod pdf;

use std::env;
use std::path::PathBuf;
use std::process::{self, Command, Stdio};

use winit::event_loop::EventLoop;

use app::App;

fn main() {
    let mut args: Vec<String> = env::args().skip(1).collect();
    let foreground = args.iter().any(|a| a == "--foreground" || a == "-f");
    args.retain(|a| a != "--foreground" && a != "-f");

    let path = args.first().map(PathBuf::from).unwrap_or_else(|| {
        eprintln!("usage: riftpdf [--foreground|-f] <file.pdf>");
        process::exit(1);
    });

    if !path.exists() {
        eprintln!("file not found: {}", path.display());
        process::exit(1);
    }

    // GUI apps normally return the shell immediately (like `open`).
    // Re-exec detached unless --foreground was passed.
    if !foreground {
        detach_and_exit(&path);
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

fn detach_and_exit(path: &std::path::Path) -> ! {
    let exe = env::current_exe().unwrap_or_else(|_| PathBuf::from("riftpdf"));
    let mut cmd = Command::new(exe);
    cmd.arg("--foreground").arg(path);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        unsafe {
            cmd.pre_exec(|| {
                // New session so the terminal closing doesn't kill the viewer.
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }

    if let Err(e) = cmd.spawn() {
        eprintln!("failed to launch: {e}");
        process::exit(1);
    }
    process::exit(0);
}
