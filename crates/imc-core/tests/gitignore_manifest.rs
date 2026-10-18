//! The manifest must respect the project's `.gitignore`, like Python does.
//!
//! Python builds its file list from `git ls-files --cached --others
//! --exclude-standard` plus `.gitignore` pathspecs, so a generated tree never
//! reaches the index. The port walked the filesystem with a hardcoded dir list
//! and indexed `/root/imc/indexmycode.egg-info/entry_points.txt`, which then
//! outranked `main/cli/main.py` for "cli entry point argument parsing".

use imc_core::ingest::build_manifest;

fn tmp(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("imc-manifest-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn write(root: &std::path::Path, rel: &str, body: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, body).unwrap();
}

#[test]
fn gitignored_trees_are_not_indexed() {
    let root = tmp("gitignore");
    std::fs::create_dir_all(root.join(".git")).unwrap(); // makes .gitignore apply
    std::fs::write(root.join(".gitignore"), "*.egg-info/\nbuild/\ndata/\n!keep/\n").unwrap();
    write(&root, "main/cli/main.py", "def main():\n    return 0\n");
    write(&root, "pkg.egg-info/entry_points.txt", "[console_scripts]\nimc = main:cli\n");
    write(&root, "build/out.py", "x = 1\n");
    write(&root, "keep/notes.py", "y = 2\n"); // not ignored (and not matched by any rule)

    let manifest = build_manifest(&root, 1_000_000, &[]).unwrap();
    let files: Vec<String> = manifest.iter().map(|f| f.rel.clone()).collect();

    assert!(files.contains(&"main/cli/main.py".to_string()), "{files:?}");
    assert!(files.contains(&"keep/notes.py".to_string()), "{files:?}");
    assert!(
        !files.iter().any(|f| f.contains("egg-info")),
        "generated package metadata must not be indexed: {files:?}"
    );
    assert!(
        !files.iter().any(|f| f.starts_with("build/")),
        "ignored build output must not be indexed: {files:?}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn without_a_gitignore_the_hardcoded_list_still_applies() {
    let root = tmp("nogitignore");
    write(&root, "src/app.py", "print(1)\n");
    write(&root, "node_modules/dep/index.js", "module.exports = 1;\n");
    write(&root, "target/debug/thing.rs", "fn main() {}\n");

    let files: Vec<String> = build_manifest(&root, 1_000_000, &[])
        .unwrap()
        .iter()
        .map(|f| f.rel.clone())
        .collect();
    assert!(files.contains(&"src/app.py".to_string()), "{files:?}");
    assert!(!files.iter().any(|f| f.starts_with("node_modules/")), "{files:?}");
    assert!(!files.iter().any(|f| f.starts_with("target/")), "{files:?}");
    let _ = std::fs::remove_dir_all(&root);
}
