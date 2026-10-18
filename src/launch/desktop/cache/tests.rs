//! Cache roundtrip and invalidation, driven through temp dirs and explicit
//! cache paths so no environment variable (never [`cache_path`]) is touched.

use super::super::keys;
use super::*;

/// An entry built by hand - no locale lookup, no file read - with its keys
/// computed exactly the way the scan computes them.
fn entry(id: &str, name: &str, names: &[&str]) -> Entry {
    let mut built = Entry {
        id: id.to_string(),
        path: PathBuf::from(format!("/applications/{id}.desktop")),
        name: name.to_string(),
        names: names.iter().map(|n| n.to_string()).collect(),
        exec: Some(format!("{id} %U")),
        keywords: Vec::new(),
        no_display: false,
        keys: Vec::new(),
    };
    built.keys = keys(&built);
    built
}

/// The one-directory directory list every test here scans.
fn one_dir(dir: &Path) -> Vec<PathBuf> {
    vec![dir.to_path_buf()]
}

#[test]
fn a_stored_cache_roundtrips_entries_with_their_keys() {
    let tmp = tempfile::tempdir().unwrap();
    let apps = tmp.path().join("applications");
    fs::create_dir(&apps).unwrap();
    fs::write(apps.join("files.desktop"), "[Desktop Entry]\nName=Files\n").unwrap();

    let entries = vec![
        entry("files", "Files", &["Files"]),
        entry("wenjian", "文件管理器", &["文件管理器", "Files"]),
    ];
    // the Chinese name and its pinyin acronym are live keys, so the JSON
    // roundtrip has to carry unicode intact
    assert!(entries[1].keys.contains(&"文件管理器".to_string()));
    assert!(entries[1].keys.contains(&"wjglq".to_string()));

    let path = tmp.path().join("app-scan.json");
    let stamps = fingerprint(&one_dir(&apps));
    store(&path, &stamps, &entries);

    assert_eq!(load(&path, &fingerprint(&one_dir(&apps))), Some(entries));
}

#[test]
fn adding_a_file_invalidates_the_cache() {
    let tmp = tempfile::tempdir().unwrap();
    let apps = tmp.path().join("applications");
    fs::create_dir(&apps).unwrap();
    fs::write(apps.join("a.desktop"), "Name=A\n").unwrap();
    let path = tmp.path().join("app-scan.json");
    let stamps = fingerprint(&one_dir(&apps));
    store(&path, &stamps, &[entry("a", "A", &["A"])]);

    fs::write(apps.join("b.desktop"), "Name=B\n").unwrap();
    assert!(load(&path, &fingerprint(&one_dir(&apps))).is_none());
}

#[test]
fn rewriting_a_file_invalidates_the_cache() {
    let tmp = tempfile::tempdir().unwrap();
    let apps = tmp.path().join("applications");
    fs::create_dir(&apps).unwrap();
    fs::write(apps.join("a.desktop"), "Name=A\n").unwrap();
    let path = tmp.path().join("app-scan.json");
    let stamps = fingerprint(&one_dir(&apps));
    store(&path, &stamps, &[entry("a", "A", &["A"])]);

    // a different length, so the miss cannot hinge on mtime granularity
    fs::write(
        apps.join("a.desktop"),
        "[Desktop Entry]\nName=A much longer label\n",
    )
    .unwrap();
    assert!(load(&path, &fingerprint(&one_dir(&apps))).is_none());
}

#[test]
fn removing_the_directory_invalidates_the_cache() {
    let tmp = tempfile::tempdir().unwrap();
    let apps = tmp.path().join("applications");
    fs::create_dir(&apps).unwrap();
    fs::write(apps.join("a.desktop"), "Name=A\n").unwrap();
    let path = tmp.path().join("app-scan.json");
    let stamps = fingerprint(&one_dir(&apps));
    store(&path, &stamps, &[entry("a", "A", &["A"])]);

    fs::remove_dir_all(&apps).unwrap();
    assert!(load(&path, &fingerprint(&one_dir(&apps))).is_none());
}

#[test]
fn a_corrupt_cache_file_is_a_miss() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("app-scan.json");
    fs::write(&path, "definitely not json, just damage").unwrap();
    assert!(load(&path, &[]).is_none());
}

#[test]
fn a_cache_from_another_version_is_a_miss() {
    let tmp = tempfile::tempdir().unwrap();
    let apps = tmp.path().join("applications");
    fs::create_dir(&apps).unwrap();
    fs::write(apps.join("a.desktop"), "Name=A\n").unwrap();
    let path = tmp.path().join("app-scan.json");
    let stamps = fingerprint(&one_dir(&apps));

    // a hand-written document whose fingerprint and entries would serve,
    // only the version is foreign
    let json = format!(
        "{{\"version\":{},\"fingerprint\":{},\"entries\":{}}}",
        VERSION + 1,
        serde_json::to_string(&stamps).unwrap(),
        serde_json::to_string(&[entry("a", "A", &["A"])]).unwrap(),
    );
    fs::write(&path, json).unwrap();
    assert!(load(&path, &stamps).is_none());
}

#[test]
fn non_desktop_files_are_invisible_to_the_fingerprint() {
    let tmp = tempfile::tempdir().unwrap();
    let apps = tmp.path().join("applications");
    fs::create_dir(&apps).unwrap();
    fs::write(apps.join("a.desktop"), "Name=A\n").unwrap();
    fs::write(apps.join("notes.txt"), "not a desktop file\n").unwrap();
    fs::write(apps.join("README"), "neither is this\n").unwrap();

    let stamps = fingerprint(&one_dir(&apps));
    let names: Vec<&str> = stamps[0].files.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, vec!["a.desktop"]);
}

#[test]
fn an_empty_first_scan_leaves_no_cache_file_behind() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("app-scan.json");
    let missing = tmp.path().join("applications");
    store(&path, &fingerprint(&[missing]), &[]);
    assert!(!path.exists());
}
