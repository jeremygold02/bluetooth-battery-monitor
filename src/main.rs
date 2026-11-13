#![windows_subsystem = "windows"]
#![allow(non_snake_case)]

mod collect;
mod devices;
mod hfp;
mod icon;
mod ipc;
mod monitor;
mod popup;
mod probe;
mod rate;
mod settings;
mod settings_window;
mod shell;
mod startup;
mod theme;
mod tray;
mod updater;
mod win;
mod windows_collectors;

fn main() -> eframe::Result {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let has = |flag: &str| arguments.iter().any(|argument| argument == flag);
    let value_after = |flag: &str| {
        arguments.iter().position(|argument| argument == flag).and_then(|index| arguments.get(index + 1))
    };
    if has("--demo-states") {
        return popup::run_demo(true, has("--light"));
    }
    if has("--demo") {
        return popup::run_demo(false, has("--light"));
    }
    if let Some(page) = value_after("--demo-settings") {
        return settings_window::run_demo(page.parse().unwrap_or(2), has("--light"));
    }
    if has("--popup") {
        return popup::run_connected(&arguments);
    }
    if has("--settings") {
        return settings_window::run_connected();
    }
    tray::run(&arguments);
    Ok(())
}
