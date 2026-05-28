#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod clipboard;
mod config;
mod log;
mod sync;
mod tray;
mod ws;

#[cfg(windows)]
fn main() -> anyhow::Result<()> {
    use config::FirstRun;

    let cfg = match config::load_or_init() {
        Ok(FirstRun::Loaded(c)) => c,
        Ok(FirstRun::TemplateWritten(p)) => {
            tray::show_dialog(
                "ClipSync — ilk açılış",
                &format!(
                    "Şablon yazıldı:\n{}\n\nLütfen ServerUrl, RoomId ve Token alanlarını düzenleyip uygulamayı tekrar başlatın.",
                    p.display()
                ),
            );
            return Ok(());
        }
        Ok(FirstRun::NeedsEdit(p)) => {
            tray::show_dialog(
                "ClipSync — config düzenlenmedi",
                &format!(
                    "{} hâlâ şablon değerleri içeriyor.\n\nLütfen düzenleyip tekrar başlatın.",
                    p.display()
                ),
            );
            return Ok(());
        }
        Err(e) => {
            tray::show_dialog(
                "ClipSync — config hatası",
                &format!("Config yüklenemedi:\n\n{:#}", e),
            );
            return Err(e);
        }
    };

    tray::run(cfg)
}

#[cfg(not(windows))]
fn main() {
    eprintln!(
        "clipsync v{} — şimdilik yalnızca Windows. Linux desteği planda.",
        env!("CARGO_PKG_VERSION")
    );
    std::process::exit(1);
}
