//! Unit tests for the bookmark import (see [`super`]).

use super::*;

/// Chrome tree with nested folders, a duplicate URL, a blank title and
/// nodes (non-http scheme, blank URL, separator) that must be dropped.
const TREE: &str = r#"{"roots": {"bar": {"type": "folder", "name": "Bookmarks bar", "children": [
    {"type": "url", "name": "Rust", "url": "https://www.rust-lang.org/"},
    {"type": "folder", "name": "Dev Tools", "children": [
      {"type": "url", "name": "GitHub", "url": "https://github.com/"},
      {"type": "folder", "name": "Deeper", "children": [
        {"type": "url", "name": "GH Issues", "url": "https://github.com/issues"}]},
      {"type": "url", "name": "Rust", "url": "https://www.rust-lang.org/"}]},
    {"type": "url", "name": "", "url": "https://www.example.com/page"},
    {"type": "url", "name": "Bad Title!@#", "url": "https://example.org/a"},
    {"type": "url", "name": "Bad Title!@#", "url": "https://example.org/b"},
    {"type": "url", "name": "Script", "url": "javascript:alert(1)"},
    {"type": "url", "name": "No Scheme", "url": "ftp://example.net/x"},
    {"type": "url", "name": "Blank", "url": "   "},
    {"type": "separator"},
    {"type": "url", "name": "Example", "url": "https://example.net/x"}]},
  "other": {"type": "folder", "name": "Other bookmarks", "children": []}}}"#;

fn entry(title: &str, url: &str, folder: &str) -> Entry {
    Entry {
        title: title.into(),
        url: url.into(),
        folder: folder.into(),
    }
}

fn dump(fresh: &[(String, String)]) -> Vec<String> {
    fresh.iter().map(|(k, v)| format!("{k}={v}")).collect()
}

#[test]
fn parse_walks_folders_and_filters_urls() {
    let got: Vec<String> = parse(TREE)
        .unwrap()
        .into_iter()
        .map(|e| format!("{}|{}|{}", e.title, e.url, e.folder))
        .collect();
    assert_eq!(
        got,
        [
            "Rust|https://www.rust-lang.org/|",
            "GitHub|https://github.com/|Dev Tools",
            "GH Issues|https://github.com/issues|Dev Tools/Deeper",
            "|https://www.example.com/page|",
            "Bad Title!@#|https://example.org/a|",
            "Bad Title!@#|https://example.org/b|",
            "Example|https://example.net/x|",
        ]
    );
}

#[test]
fn parse_and_load_edge_cases() {
    assert!(parse("{}").unwrap().is_empty());
    assert!(parse(r#"{"roots": []}"#).unwrap().is_empty());
    assert!(parse("nope").unwrap_err().starts_with("invalid bookmarks"));

    let children: Vec<serde_json::Value> = (0..MAX_IMPORT + 5)
        .map(|i| serde_json::json!({"type": "url", "name": format!("t{i}"), "url": format!("https://e.example/{i}")}))
        .collect();
    let tree = serde_json::json!({"roots": {"bar": {"type": "folder", "children": children}}});
    assert_eq!(parse(&tree.to_string()).unwrap().len(), MAX_IMPORT);

    let dir = tempfile::tempdir().unwrap();
    let err = load(&dir.path().join("Bookmarks")).unwrap_err();
    assert!(err.starts_with("cannot read bookmarks"), "{err}");
    let bad = dir.path().join("bad.json");
    fs::write(&bad, "nope").unwrap();
    assert!(load(&bad).unwrap_err().starts_with("invalid bookmarks"));
}

#[test]
fn profile_scan_and_env_override() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    for profile in ["Zed", "Alpha", "Default"] {
        let profile_dir = dir.path().join(profile);
        fs::create_dir_all(&profile_dir).unwrap();
        fs::write(profile_dir.join("Bookmarks"), "{}").unwrap();
    }
    fs::create_dir(dir.path().join("NoFile")).unwrap();
    let names: Vec<String> = profiles_in(dir.path())
        .iter()
        .map(|p| {
            p.parent()
                .unwrap()
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    assert_eq!(names, vec!["Default", "Alpha", "Zed"]);

    let prev = std::env::var_os("XC_CHROME_BOOKMARKS");
    std::env::set_var("XC_CHROME_BOOKMARKS", "/no/such/Bookmarks");
    assert_eq!(find_file(), Some(PathBuf::from("/no/such/Bookmarks")));
    match prev {
        Some(v) => std::env::set_var("XC_CHROME_BOOKMARKS", v),
        None => std::env::remove_var("XC_CHROME_BOOKMARKS"),
    }
}

#[test]
fn plan_slugs_collisions_and_existing_key_protection() {
    let fresh = plan(
        &BTreeMap::new(),
        &[
            entry("Hello, World!", "https://a.example/1", ""),
            entry("", "https://www.example.com/page", ""),
            entry("!!!", "https://docs.rs/x", ""),
            entry("  ", "https://", ""),
        ],
    );
    assert_eq!(
        dump(&fresh),
        [
            "hello-world=https://a.example/1",
            "example.com=https://www.example.com/page",
            "docs.rs=https://docs.rs/x",
            "bookmark=https://",
        ]
    );

    // A taken plain key is qualified by the folder chain, then numbered;
    // existing keys are never returned.
    let existing = BTreeMap::from([
        ("github".to_string(), "x".to_string()),
        ("dev-tools-github".to_string(), "y".to_string()),
    ]);
    let fresh = plan(
        &existing,
        &[
            entry("GitHub", "https://github.com/a", "Dev Tools"),
            entry("GitHub", "https://github.com/b", ""),
        ],
    );
    assert_eq!(
        dump(&fresh),
        [
            "github-2=https://github.com/a",
            "github-3=https://github.com/b"
        ]
    );
    assert!(fresh.iter().all(|(k, _)| !existing.contains_key(k)));

    let one = plan(
        &BTreeMap::from([("github".to_string(), "x".to_string())]),
        &[entry("GitHub", "u", "Dev Tools")],
    );
    assert_eq!(dump(&one), ["dev-tools-github=u"]);

    // A bookmark whose URL is already registered under another key is done.
    let none = plan(
        &BTreeMap::from([("baidu".to_string(), "https://www.baidu.com".to_string())]),
        &[entry("Baidu", "https://www.baidu.com", "Search")],
    );
    assert!(none.is_empty(), "same URL, nothing to add: {none:?}");
}

#[test]
fn merge_into_never_overwrites_and_only_adds_missing_keys() {
    let mut aliases = crate::alias::defaults();
    let fresh = [("x".to_string(), "https://x.example".to_string())];
    assert_eq!(merge_into(&mut aliases, "br", &fresh).unwrap(), 1);
    assert_eq!(aliases.len(), 3, "edited in place, no copy appears");
    let br = aliases.iter().find(|d| d.name == "br").unwrap();
    assert_eq!(
        br.linux.as_deref(),
        Some("xdg-open {input} >/dev/null 2>&1 &")
    );
    let rows: Vec<String> = br
        .shortcuts
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect();
    assert_eq!(
        rows,
        [
            "baidu=https://www.baidu.com",
            "gm=https://mail.google.com",
            "x=https://x.example",
        ]
    );

    let overwrite = [("x".to_string(), "https://other.example".to_string())];
    assert_eq!(merge_into(&mut aliases, "br", &overwrite).unwrap(), 0);
    assert_eq!(
        aliases
            .iter()
            .find(|d| d.name == "br")
            .unwrap()
            .shortcuts
            .get("x")
            .map(String::as_str),
        Some("https://x.example")
    );
    assert_eq!(aliases.len(), 3, "existing entries stay in place");
    assert_eq!(
        merge_into(&mut aliases, "ghost", &fresh).unwrap_err(),
        "alias not found: ghost"
    );
    assert_eq!(aliases.len(), 3, "a missing target changes nothing");
}

#[test]
fn plan_and_merge_is_re_runnable() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("Bookmarks");
    fs::write(&path, TREE).unwrap();

    let mut aliases = crate::alias::defaults();
    assert_eq!(plan_and_merge(&mut aliases, "br", &path).unwrap(), 7);
    let br = aliases.iter().find(|d| d.name == "br").unwrap();
    assert_eq!(
        br.shortcuts.get("rust").map(String::as_str),
        Some("https://www.rust-lang.org/")
    );
    assert_eq!(
        br.shortcuts.get("bad-title-2").map(String::as_str),
        Some("https://example.org/b")
    );
    let after_first = br.shortcuts.len();

    assert_eq!(plan_and_merge(&mut aliases, "br", &path).unwrap(), 0);
    assert_eq!(
        aliases
            .iter()
            .find(|d| d.name == "br")
            .unwrap()
            .shortcuts
            .len(),
        after_first,
        "re-runs add nothing"
    );
    assert_eq!(
        plan_and_merge(&mut aliases, "ghost", &path).unwrap_err(),
        "alias not found: ghost"
    );
}
