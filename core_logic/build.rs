use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn collect_json_files(dir: &Path, files: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_json_files(&path, files)?;
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("json") {
            files.push(path);
        }
    }
    Ok(())
}

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("missing CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir
        .parent()
        .expect("core_logic crate should have a parent directory");
    let skills_root = repo_root.join("skills");
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("missing OUT_DIR"));
    let out_file = out_dir.join("embedded_skills.rs");

    println!("cargo:rerun-if-changed={}", skills_root.display());

    let mut files = Vec::new();
    if skills_root.exists() {
        collect_json_files(&skills_root, &mut files).expect("failed to scan skills directory");
        files.sort();
        for file in &files {
            println!("cargo:rerun-if-changed={}", file.display());
        }
    }

    let mut generated = String::from("pub fn embedded_skills() -> &'static [(&'static str, &'static str)] {\n    &[\n");
    for file in files {
        let relative = file
            .strip_prefix(repo_root)
            .unwrap_or(file.as_path())
            .to_string_lossy()
            .to_string();
        let absolute = file.to_string_lossy().to_string();
        generated.push_str(&format!(
            "        ({relative:?}, include_str!({absolute:?})),\n"
        ));
    }
    generated.push_str("    ]\n}\n");

    fs::write(&out_file, generated).expect("failed to write embedded_skills.rs");
}
