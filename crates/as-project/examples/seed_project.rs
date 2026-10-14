//! Dev tool: create a project with media in a (scratch) data folder, for
//! end-to-end runs of the real app (`AUTOSCENE_DATA_DIR=<data>`).
//! `cargo run -p as-project --example seed_project -- <data_dir> <library_root> <title> <live|anime> <files...>`

use as_api::{AppSettings, ContentKind, NewProject, ProjectKind};
use as_core::AppPaths;
use as_project::Library;

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    assert!(a.len() >= 5, "usage: seed_project <data_dir> <library_root> <title> <live|anime> <files...>");
    let lib = Library::open(&AppPaths::at(&a[0])).unwrap();
    let mut s = lib.settings().unwrap();
    s.library_root = a[1].clone();
    lib.update_settings(&AppSettings { ..s }).unwrap();
    let content = if a[3] == "anime" { ContentKind::Anime } else { ContentKind::Live };
    let (summary, _, added) = lib
        .create_project(&NewProject { title: a[2].clone(), kind: ProjectKind::Series, content, files: a[4..].to_vec() })
        .unwrap();
    println!("{} {} ({} media)", summary.id, summary.dir, added.len());
}
