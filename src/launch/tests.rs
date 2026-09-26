//! Launch-layer tests: template detection, the seeded def, resolution, the
//! macOS URL fallback and the environment-independent spawn helpers.

use super::*;

fn entry(id: &str, name: &str, path: &str) -> desktop::Entry {
    desktop::Entry {
        id: id.to_string(),
        path: std::path::PathBuf::from(path),
        name: name.to_string(),
        names: vec![name.to_string()],
        exec: Some(format!("{path} %U")),
        keywords: vec![],
        no_display: false,
    }
}

fn wechat() -> desktop::Entry {
    let mut e = entry("wechat", "WeChat", "/usr/share/applications/wechat.desktop");
    e.names.push("微信".to_string());
    e
}

fn files() -> desktop::Entry {
    entry(
        "dolphin",
        "文件管理器",
        "/usr/share/applications/dolphin.desktop",
    )
}

#[test]
fn template_is_detected_exactly_and_tolerates_padding() {
    assert!(is_native(TEMPLATE));
    assert!(is_native("  @native app  "));
    assert!(!is_native("@native app extra"));
    assert!(!is_native("@native clipboard"));
    assert!(!is_native(""));
    assert_ne!(
        TEMPLATE,
        crate::clipboard::TEMPLATE,
        "the two native backends must not collide"
    );
}

#[test]
fn default_def_seeds_the_app_alias() {
    let def = default_def();
    assert_eq!(def.name, "app");
    assert_eq!(def.linux.as_deref(), Some(TEMPLATE));
    assert_eq!(def.macos.as_deref(), Some(TEMPLATE));
    assert!(def.triggers.is_empty(), "the name is the only trigger");
    assert!(
        def.shortcuts.is_empty(),
        "the input is the application name"
    );
}

#[test]
fn describe_pairs_label_and_source() {
    let target = Resolved {
        label: "WeChat".to_string(),
        source: "/usr/share/applications/wechat.desktop".to_string(),
        exec: None,
    };
    assert_eq!(
        describe(&target),
        "WeChat (/usr/share/applications/wechat.desktop)"
    );
}

#[test]
fn resolve_from_matches_exact_names_and_pinyin_initials() {
    let path = "/usr/share/applications/wechat.desktop";
    assert_eq!(
        resolve_from(&[wechat()], "wechat"),
        Match::One(Resolved {
            label: "WeChat".to_string(),
            source: path.to_string(),
            exec: Some(format!("{path} %U")),
        })
    );
    // The pinyin acronym reaches the Chinese name; the acronym of WeChat's
    // name reaches the entry too.
    assert_eq!(
        resolve_from(&[files()], "wjglq"),
        Match::One(Resolved {
            label: "文件管理器".to_string(),
            source: "/usr/share/applications/dolphin.desktop".to_string(),
            exec: Some("/usr/share/applications/dolphin.desktop %U".to_string()),
        })
    );
    assert!(matches!(
        resolve_from(&[wechat(), files()], "wx"),
        Match::One(target) if target.label == "WeChat"
    ));
}

#[test]
fn resolve_from_reports_ambiguity_only_for_distinct_labels() {
    let chrome = entry("chrome", "Chrome", "/apps/chrome.desktop");
    let chromium = entry("chromium", "Chromium", "/apps/chromium.desktop");
    assert_eq!(
        resolve_from(&[chrome.clone(), chromium.clone()], "chrom"),
        Match::Ambiguous(vec!["Chrome".to_string(), "Chromium".to_string()])
    );
    // Same label twice: one name, one answer, whichever source is first.
    let copy = entry("chrome-beta", "Chrome", "/apps/chrome-beta.desktop");
    assert!(matches!(
        resolve_from(&[copy, chrome], "chrome"),
        Match::One(target) if target.label == "Chrome"
    ));
}

#[test]
fn resolve_from_returns_none_for_garbage_and_empty_input() {
    assert_eq!(resolve_from(&[wechat()], "zzzz"), Match::None);
    assert_eq!(resolve_from(&[wechat()], "  "), Match::None);
    assert_eq!(resolve_from(&[], "wechat"), Match::None);
}

#[test]
fn launch_requires_an_app_name() {
    assert_eq!(
        launch("", Platform::Linux),
        Outcome::Failed("app name required".to_string())
    );
    assert_eq!(
        launch("   ", Platform::Macos),
        Outcome::Failed("app name required".to_string())
    );
}

#[test]
fn ambiguous_message_lists_at_most_three_labels() {
    let two = vec!["A".to_string(), "B".to_string()];
    assert_eq!(ambiguous_message("x", &two), "x matches 2 apps: A, B");
    let four = ["A", "B", "C", "D"].map(str::to_string).to_vec();
    assert_eq!(
        ambiguous_message("xx", &four),
        "xx matches 4 apps: A, B, C, ..."
    );
}

#[test]
fn strip_field_codes_drops_placeholders_but_keeps_literal_percent() {
    assert_eq!(strip_field_codes("/usr/bin/code %F"), "/usr/bin/code");
    assert_eq!(strip_field_codes("app %U --flag %f"), "app  --flag");
    assert_eq!(strip_field_codes("echo 100%%"), "echo 100%");
    assert_eq!(strip_field_codes("/usr/bin/simple"), "/usr/bin/simple");
}

#[test]
fn macos_commands_open_the_bundle_or_url() {
    let target = Resolved {
        label: "Safari".to_string(),
        source: "/Applications/Safari.app".to_string(),
        exec: None,
    };
    let command = command_for(&target, Platform::Macos).expect("open is always there");
    assert_eq!(command.get_program(), "open");
    assert_eq!(
        command.get_args().collect::<Vec<_>>(),
        vec!["/Applications/Safari.app"]
    );
}

#[test]
fn url_fallback_passes_urls_through_on_both_platforms() {
    let url = "https://x.example/page";
    for platform in [Platform::Macos, Platform::Linux] {
        assert_eq!(
            url_fallback(url, platform, Match::None),
            Match::One(Resolved {
                label: url.to_string(),
                source: url.to_string(),
                exec: None,
            }),
            "{platform:?} passes URLs through"
        );
        assert_eq!(
            url_fallback("zzz", platform, Match::None),
            Match::None,
            "{platform:?} keeps non-URLs unresolved"
        );
    }
    // A URL with a real match behind it stays that match: the passthrough
    // is a fallback, not an override.
    let existing = resolve_from(&[wechat()], "wechat");
    assert!(matches!(&existing, Match::One(target) if target.label == "WeChat"));
    assert_eq!(
        url_fallback(url, Platform::Linux, existing.clone()),
        existing
    );
}

#[test]
fn url_targets_open_with_xdg_open_then_gio() {
    let target = Resolved {
        label: "https://wx.qq.com".to_string(),
        source: "https://wx.qq.com".to_string(),
        exec: None,
    };
    assert!(is_url(&target), "a URL-sourced target is a URL target");
    let entry_target = Resolved {
        label: "WeChat".to_string(),
        source: "/usr/share/applications/wechat.desktop".to_string(),
        exec: None,
    };
    assert!(!is_url(&entry_target), "a desktop file is not a URL");

    assert_eq!(URL_OPENERS, ["xdg-open", "gio"], "xdg-open is preferred");
    let xdg = url_command(&target, "xdg-open");
    assert_eq!(xdg.get_program(), "xdg-open");
    assert_eq!(
        xdg.get_args().collect::<Vec<_>>(),
        vec!["https://wx.qq.com"]
    );
    let gio = url_command(&target, "gio");
    assert_eq!(gio.get_program(), "gio");
    assert_eq!(
        gio.get_args().collect::<Vec<_>>(),
        vec!["open", "https://wx.qq.com"]
    );
}
