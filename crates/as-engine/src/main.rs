//! `autoscene-engine`: the analysis engine for headless use (the app runs
//! the same code as `AutoScene --engine`).

fn main() {
    std::process::exit(as_engine::main_from_args(std::env::args_os().skip(1)));
}
