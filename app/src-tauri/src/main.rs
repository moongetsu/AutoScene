// Prevents an extra console window on Windows in release. Do not remove.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // `AutoScene --engine …` is the analysis engine, which the app starts as a
    // process of its own (see `as_engine`).
    let mut args = std::env::args_os().skip(1);
    if args.next().is_some_and(|a| a == "--engine") {
        std::process::exit(as_engine::main_from_args(args));
    }
    autoscene_app::run();
}
