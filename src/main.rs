use std::fs::{File, OpenOptions};
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;

use mimic::app::MimicApp;
use mimic::controller;
use mimic::paths;
use mimic::theme;

fn main() -> eframe::Result<()> {
    let initial_file = match parse_args() {
        Ok(file) => file,
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(2);
        }
    };

    if std::env::var_os("WAYLAND_DISPLAY").is_none() {
        eprintln!(
            "Mimic requires a Wayland session (KDE Plasma). WAYLAND_DISPLAY is not set.\n\
             Recording and playback are implemented with Linux evdev/uinput and do not use X11."
        );
        std::process::exit(1);
    }

    let _instance_lock = match acquire_instance_lock() {
        Ok(file) => file,
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(1);
        }
    };

    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_app_id(paths::app_id())
            .with_title("Mimic")
            .with_inner_size([650.0, 590.0])
            .with_min_inner_size([530.0, 500.0])
            .with_icon(theme::window_icon()),
        ..Default::default()
    };

    eframe::run_native(
        "Mimic",
        options,
        Box::new(move |cc| {
            theme::install(&cc.egui_ctx);
            let controller = controller::spawn(cc.egui_ctx.clone(), initial_file);
            Ok(Box::new(MimicApp::new(controller)))
        }),
    )
}

fn parse_args() -> Result<Option<PathBuf>, String> {
    let mut file = None;
    for arg in std::env::args_os().skip(1) {
        let text = arg.to_string_lossy();
        match text.as_ref() {
            "-h" | "--help" => {
                print_help();
                std::process::exit(0);
            }
            "-V" | "--version" => {
                println!("Mimic {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            _ if text.starts_with('-') => {
                return Err(format!("Unknown option: {text}\n\n{}", short_usage()));
            }
            _ => {
                if file.is_some() {
                    return Err(format!(
                        "Only one recording can be opened at a time.\n\n{}",
                        short_usage()
                    ));
                }
                file = Some(PathBuf::from(arg));
            }
        }
    }
    Ok(file)
}

fn short_usage() -> &'static str {
    "Usage: mimic [RECORDING.mimic]"
}

fn print_help() {
    println!(
        "Mimic {} — a TinyTask-style keyboard and mouse macro recorder\n\
         for KDE Plasma on Wayland.\n\n\
         {}\n\n\
         Options:\n\
           -h, --help       Show this help\n\
           -V, --version    Show the version\n\n\
         Recording uses Linux evdev and playback uses uinput. The GUI never runs as root.\n\
         See README.md for the one-time permission setup.",
        env!("CARGO_PKG_VERSION"),
        short_usage()
    );
}

/// Uses an advisory lock in the user's runtime directory so two Mimic
/// instances cannot drive input at the same time.
fn acquire_instance_lock() -> Result<nix::fcntl::Flock<File>, String> {
    use nix::fcntl::{Flock, FlockArg};

    let directory = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let uid = nix::unistd::Uid::current();
    let path = directory.join(format!("mimic-{uid}.lock"));
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(&path)
        .map_err(|error| {
            format!(
                "Could not open Mimic's instance lock at {}: {error}",
                path.display()
            )
        })?;
    Flock::lock(file, FlockArg::LockExclusiveNonblock)
        .map_err(|_| "Mimic is already running in this session.".to_owned())
}
