//! Platform-aware command tests for `alias.rs` (`platform_command` and
//! `set_command`), split into their own small file so the extracted main test
//! module stays within the size budget.

use super::*;

fn def_with(name: &str, linux: Option<&str>, macos: Option<&str>) -> AliasDef {
    AliasDef {
        name: name.to_string(),
        triggers: vec![],
        linux: linux.map(str::to_string),
        macos: macos.map(str::to_string),
        shortcuts: BTreeMap::new(),
    }
}

#[test]
fn set_command_touches_only_the_named_platform() {
    let mut user = vec![def_with(
        "t",
        Some("xdg-open {input}"),
        Some("open {input}"),
    )];
    set_command(&mut user, "t", Platform::Linux, "printf %s {input}").unwrap();
    assert_eq!(user[0].linux.as_deref(), Some("printf %s {input}"));
    assert_eq!(
        user[0].macos.as_deref(),
        Some("open {input}"),
        "macos stays byte-identical"
    );

    set_command(&mut user, "t", Platform::Macos, "open -a Safari {input}").unwrap();
    assert_eq!(
        user[0].linux.as_deref(),
        Some("printf %s {input}"),
        "linux stays byte-identical"
    );
    assert_eq!(user[0].macos.as_deref(), Some("open -a Safari {input}"));
    assert_eq!(user.len(), 1, "no duplicate definition appears");
}

#[test]
fn set_command_edits_a_seeded_alias_and_reports_unknowns() {
    let mut user = defaults();
    set_command(&mut user, "br", Platform::Macos, "open -a Safari {input}").unwrap();
    let br = resolve(&user, "br").expect("br still resolves");
    assert_eq!(br.macos.as_deref(), Some("open -a Safari {input}"));
    assert_eq!(
        br.linux.as_deref(),
        Some("xdg-open {input} >/dev/null 2>&1 &"),
        "the seeded linux command stays put"
    );
    assert_eq!(br.shortcuts.len(), 2, "the seeded shortcuts stay put");
    let err = set_command(&mut user, "ghost", Platform::Linux, "a").unwrap_err();
    assert_eq!(err, "alias not found: ghost");
}

#[test]
fn platform_command_picks_the_requested_platform_and_blank_is_none() {
    let blank = def_with("t", Some("   "), Some("open {input}"));
    assert_eq!(
        platform_command(&blank, Platform::Linux),
        None,
        "whitespace-only counts as not configured"
    );
    assert_eq!(
        platform_command(&blank, Platform::Macos),
        Some("open {input}")
    );

    let none = def_with("t", None, None);
    assert_eq!(platform_command(&none, Platform::Linux), None);
    assert_eq!(platform_command(&none, Platform::Macos), None);

    let both = def_with("t", Some("xdg-open {input}"), Some("open {input}"));
    assert_eq!(
        platform_command(&both, Platform::Linux),
        Some("xdg-open {input}"),
        "no cross-platform fallback in the table"
    );
    assert_eq!(
        platform_command(&both, Platform::Macos),
        Some("open {input}")
    );
}
