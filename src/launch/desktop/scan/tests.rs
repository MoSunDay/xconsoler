//! Directory scanning: precedence, Hidden masking and runnability checks,
//! driven through temp dirs so no environment variable is touched.

use super::*;

fn write(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, body).unwrap();
    path
}

const HIGH: &str = "[Desktop Entry]\nName=High\nExec=high %U\n";
const LOW: &str = "[Desktop Entry]\nName=Low\nExec=low %U\n";
const HIDDEN: &str = "[Desktop Entry]\nName=Gone\nExec=gone %U\nHidden=true\n";

fn dirs(a: &Path, b: &Path) -> Vec<PathBuf> {
    vec![a.to_path_buf(), b.to_path_buf()]
}

#[test]
fn the_first_directory_wins_for_the_same_id() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    write(a.path(), "x.desktop", HIGH);
    write(b.path(), "x.desktop", LOW);

    let entries = scan_dirs(&dirs(a.path(), b.path()));
    assert_eq!(entries.len(), 1, "one id, one entry: {entries:?}");
    assert_eq!(entries[0].name, "High");
    assert_eq!(entries[0].path, a.path().join("x.desktop"));
}

#[test]
fn hidden_in_a_higher_directory_masks_the_id_everywhere() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    write(a.path(), "x.desktop", HIDDEN);
    write(b.path(), "x.desktop", LOW);

    assert!(scan_dirs(&dirs(a.path(), b.path())).is_empty());
}

#[test]
fn hidden_in_a_lower_directory_cannot_mask_a_visible_entry() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    write(a.path(), "x.desktop", HIGH);
    write(b.path(), "x.desktop", HIDDEN);

    let entries = scan_dirs(&dirs(a.path(), b.path()));
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "High");
}

#[test]
fn entries_that_cannot_run_are_skipped_or_kept_by_dbus() {
    let a = tempfile::tempdir().unwrap();
    write(
        a.path(),
        "no-exec.desktop",
        "[Desktop Entry]\nName=No Exec\n",
    );
    write(
        a.path(),
        "blank-exec.desktop",
        "[Desktop Entry]\nName=Blank\nExec=   \n",
    );
    write(
        a.path(),
        "dbus.desktop",
        "[Desktop Entry]\nName=DBus App\nDBusActivatable=true\n",
    );
    write(
        a.path(),
        "dbus-blank.desktop",
        "[Desktop Entry]\nName=DBus Blank\nExec=\nDBusActivatable=true\n",
    );

    let entries = scan_dirs(&[a.path().to_path_buf()]);
    let ids: Vec<&str> = entries.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(ids, vec!["dbus-blank", "dbus"], "{entries:?}");
}

#[test]
fn try_exec_must_name_a_binary_on_path() {
    let a = tempfile::tempdir().unwrap();
    write(
        a.path(),
        "missing.desktop",
        "[Desktop Entry]\nName=Missing\nExec=x\nTryExec=/no/such/binary_xconsoler\n",
    );
    write(
        a.path(),
        "present.desktop",
        "[Desktop Entry]\nName=Present\nExec=sh -c true\nTryExec=sh\n",
    );

    let entries = scan_dirs(&[a.path().to_path_buf()]);
    let ids: Vec<&str> = entries.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(ids, vec!["present"], "{entries:?}");
}

#[test]
fn only_top_level_desktop_files_are_read() {
    let a = tempfile::tempdir().unwrap();
    fs::create_dir(a.path().join("nested")).unwrap();
    write(&a.path().join("nested"), "deep.desktop", HIGH);
    write(a.path(), "notes.txt", "not a desktop file");
    write(a.path(), "top.desktop", LOW);

    let entries = scan_dirs(&[a.path().to_path_buf()]);
    let ids: Vec<&str> = entries.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(ids, vec!["top"], "{entries:?}");
}

#[test]
fn no_display_is_listed_but_flagged_while_hidden_is_dropped() {
    let a = tempfile::tempdir().unwrap();
    write(
        a.path(),
        "menu-hidden.desktop",
        "[Desktop Entry]\nName=Menu Hidden\nExec=x\nNoDisplay=true\n",
    );
    write(a.path(), "hidden.desktop", HIDDEN);

    let entries = scan_dirs(&[a.path().to_path_buf()]);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].id, "menu-hidden");
    assert!(entries[0].no_display);
}

#[test]
fn a_missing_directory_is_silently_skipped() {
    let a = tempfile::tempdir().unwrap();
    write(a.path(), "top.desktop", LOW);
    let missing = a.path().join("does-not-exist");

    let entries = scan_dirs(&[missing, a.path().to_path_buf()]);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "Low");
}
