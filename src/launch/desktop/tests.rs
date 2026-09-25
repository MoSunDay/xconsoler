//! Parse, key and score coverage for `desktop.rs`, on fixtures modelled on a
//! real machine's desktop files.

use super::*;

/// One entry with an explicit names list: the first name is the label.
fn entry(id: &str, names: &[&str], exec: &str, keywords: &[&str]) -> Entry {
    Entry {
        id: id.to_string(),
        path: PathBuf::from(format!("/apps/{id}.desktop")),
        name: names.first().copied().unwrap_or(id).to_string(),
        names: names.iter().map(|name| name.to_string()).collect(),
        exec: Some(exec.to_string()),
        keywords: keywords.iter().map(|k| k.to_string()).collect(),
        no_display: false,
    }
}

fn has(keys: &[String], key: &str) -> bool {
    keys.iter().any(|k| k == key)
}

const WECHAT: &str = "\
[Desktop Entry]
Name=WeChat
Name[zh_CN]=微信
Comment=chat
Exec=/opt/wechat/wechat %U
Keywords=IM;Chat;
NoDisplay=false
";

#[test]
fn parse_reads_the_main_group_and_localized_names() {
    let path = Path::new("/usr/share/applications/wechat.desktop");
    let e = parse_with_locale(WECHAT, "wechat", path, Some("zh_CN"));
    assert_eq!(e.id, "wechat");
    assert_eq!(e.path, path);
    assert_eq!(e.name, "微信", "the locale-specific name is the label");
    assert_eq!(
        e.names,
        vec!["微信".to_string(), "WeChat".to_string()],
        "the label comes first, then the plain name"
    );
    assert_eq!(e.exec.as_deref(), Some("/opt/wechat/wechat %U"));
    assert_eq!(e.keywords, vec!["IM".to_string(), "Chat".to_string()]);
    assert!(!e.no_display);
}

#[test]
fn parse_keeps_only_the_users_own_translations() {
    // A real entry: XFCE ships a translation for every locale it speaks.
    let text = "\
[Desktop Entry]
Name=Window Manager Tweaks
Name[de]=Feineinstellungen der Fensterverwaltung
Name[ja]=ウィンドウマネージャー (詳細)
Name[zh_CN]=窗口管理器微调
Keywords[de]=fenster;
Keywords[ja]=ウィンドウ;
Keywords[zh_CN]=窗口;行为;
Exec=xfwm4-tweaks-settings
";
    let e = parse_with_locale(
        text,
        "xfce-wmtweaks-settings",
        Path::new("/x.desktop"),
        Some("zh_CN"),
    );
    assert_eq!(e.name, "窗口管理器微调");
    assert_eq!(
        e.names,
        vec![
            "窗口管理器微调".to_string(),
            "Window Manager Tweaks".to_string()
        ]
    );
    assert_eq!(e.keywords, vec!["窗口".to_string(), "行为".to_string()]);
    let keys = keys(&e);
    assert!(
        !has(&keys, "xx"),
        "the Japanese 詳細 would spell xx and hide 小小备忘录: {keys:?}"
    );
    assert!(!has(&keys, "feineinstellungenderfensterverwaltung"));
    assert!(has(&keys, "ckglqwd"), "窗口管理器微调 still spells itself");
}

#[test]
fn parse_accepts_a_language_only_translation() {
    let text = "[Desktop Entry]\nName=WeChat\nName[zh]=微信\nExec=wechat\n";
    let e = parse_with_locale(text, "wechat", Path::new("/w.desktop"), Some("zh_CN"));
    assert_eq!(
        e.name, "微信",
        "zh_CN falls back to the bare zh translation"
    );
    assert_eq!(e.names, vec!["微信".to_string(), "WeChat".to_string()]);
    let plain = parse_with_locale(text, "wechat", Path::new("/w.desktop"), Some("de_DE"));
    assert_eq!(
        plain.name, "WeChat",
        "an unrelated locale keeps the plain name"
    );
    assert_eq!(plain.names, vec!["WeChat".to_string()]);
}

#[test]
fn parse_falls_back_to_the_plain_name_then_the_id() {
    let path = Path::new("/x.desktop");
    assert_eq!(
        parse_with_locale(WECHAT, "wechat", path, None).name,
        "WeChat"
    );
    assert_eq!(
        parse_with_locale(WECHAT, "wechat", path, Some("de_DE")).name,
        "WeChat",
        "an untranslated locale keeps the plain name"
    );
    let no_name = "[Desktop Entry]\nExec=app %F\n";
    assert_eq!(parse(no_name, "app-id", path).name, "app-id");
}

#[test]
fn parse_flags_hidden_and_no_display_and_keeps_field_codes_raw() {
    let hidden = "[Desktop Entry]\nName=X\nExec=x\nHidden=true\nNoDisplay=false\n";
    assert!(parse(hidden, "x", Path::new("/x.desktop")).no_display);
    let menu_hidden = "[Desktop Entry]\nName=X\nExec=x\nNoDisplay=True\n";
    assert!(parse(menu_hidden, "x", Path::new("/x.desktop")).no_display);
    let visible = parse(WECHAT, "wechat", Path::new("/w.desktop"));
    assert!(
        visible.exec.unwrap().contains("%U"),
        "field codes stay for the launcher to strip"
    );
}

#[test]
fn parse_ignores_desktop_action_groups() {
    let text = "\
[Desktop Entry]
Name=Main
Exec=main
[Desktop Action new]
Name=New Window
Exec=main --new
";
    let e = parse(text, "main", Path::new("/m.desktop"));
    assert_eq!(e.names, vec!["Main".to_string()]);
    assert_eq!(e.exec.as_deref(), Some("main"));
}

#[test]
fn normalize_locale_strips_codeset_and_modifier() {
    assert_eq!(normalize_locale("zh_CN.UTF-8"), Some("zh_CN".to_string()));
    assert_eq!(normalize_locale("de_DE@euro"), Some("de_DE".to_string()));
    assert_eq!(normalize_locale(" en_US "), Some("en_US".to_string()));
    assert_eq!(normalize_locale("C"), None);
    assert_eq!(normalize_locale("POSIX"), None);
    assert_eq!(normalize_locale(""), None);
}

#[test]
fn keys_cover_names_words_acronyms_keywords_and_the_executable() {
    let wechat = entry(
        "wechat",
        &["WeChat", "微信"],
        "/opt/wechat/wechat %U",
        &["IM", "Chat"],
    );
    let keys = keys(&wechat);
    for key in ["wechat", "微信", "wx", "im", "chat"] {
        assert!(has(&keys, key), "missing key {key:?} in {keys:?}");
    }
    assert_eq!(
        keys.iter().filter(|k| *k == "wechat").count(),
        1,
        "id, name and exec base name collapse into one key"
    );
}

#[test]
fn keys_split_names_on_punctuation_and_remove_separators() {
    let chrome = entry(
        "google-chrome",
        &["Google Chrome"],
        "/usr/bin/google-chrome-stable %U",
        &[],
    );
    let keys = keys(&chrome);
    for key in ["google chrome", "googlechrome", "google", "chrome"] {
        assert!(has(&keys, key), "missing key {key:?} in {keys:?}");
    }
    assert!(
        has(&keys, "google-chrome-stable"),
        "the executable's base name is a key: {keys:?}"
    );
}

#[test]
fn keys_keep_vendor_ids_and_their_segments() {
    let files = entry(
        "org.gnome.Nautilus",
        &["Files"],
        "/usr/bin/nautilus %U",
        &[],
    );
    let keys = keys(&files);
    for key in ["org.gnome.nautilus", "org", "gnome", "nautilus", "files"] {
        assert!(has(&keys, key), "missing key {key:?} in {keys:?}");
    }
}

#[test]
fn score_ranks_exact_over_prefix_over_substring() {
    let chrome = entry(
        "google-chrome",
        &["Google Chrome"],
        "/usr/bin/google-chrome-stable %U",
        &[],
    );
    assert_eq!(score(&chrome, "chrome"), 4, "a whole name word is exact");
    assert_eq!(score(&chrome, "googl"), 3, "prefix of google");
    assert_eq!(score(&chrome, "goo"), 3, "two-letter tokens may prefix");
    // "chrom" prefixes the word key "chrome"; a true substring needs to miss
    // every word start to score 2.
    assert_eq!(score(&chrome, "chrom"), 3);
    assert_eq!(score(&chrome, "oogle"), 2, "substring, not a prefix");
    assert_eq!(score(&chrome, "g"), 0, "one-letter keys never match");
    assert_eq!(score(&chrome, "zzz"), 0);
}

#[test]
fn score_reaches_pinyin_initials() {
    let files = entry("dolphin", &["文件管理器"], "/usr/bin/dolphin %U", &[]);
    assert_eq!(score(&files, "wjglq"), 4, "the acronym is a key of its own");
    assert_eq!(score(&files, "wjg"), 3, "a partial acronym still prefixes");
    let memo = entry(
        "electron-memo",
        &["小小备忘录"],
        "/usr/bin/electron-memo %U",
        &[],
    );
    assert_eq!(score(&memo, "xx"), 3, "prefix of the xxbwl acronym");
    assert_eq!(score(&memo, "xxbwl"), 4);
    assert_eq!(score(&memo, "memo"), 2, "the id's tail is a substring");
}

#[test]
fn score_folds_case_and_whitespace_runs() {
    let chrome = entry(
        "google-chrome",
        &["Google Chrome"],
        "/usr/bin/google-chrome-stable %U",
        &[],
    );
    // A whole name is the best match there is, folding included.
    assert_eq!(score(&chrome, "  GOOGLE   chrome "), 5);
    assert_eq!(score(&chrome, "zzqq"), 0, "no key carries that");
    assert_eq!(score(&chrome, ""), 0);
}

#[test]
fn best_returns_the_top_scoring_group_with_visible_entries_first() {
    let mut hidden = entry("dash", &["Chrome"], "/dash", &[]);
    hidden.no_display = true;
    let visible = entry("chrome", &["Chrome"], "/chrome", &[]);
    let entries = vec![hidden, visible];

    let top = best(&entries, "chrome");
    assert_eq!(top.len(), 2, "both match equally well");
    assert_eq!(top[0].id, "chrome", "the visible entry leads");
    assert_eq!(top[1].id, "dash", "a NoDisplay entry is still launchable");
    assert!(
        best(&entries, "ff").is_empty(),
        "nothing scores, nothing out"
    );
    assert!(
        best(&entries, "  ").is_empty(),
        "empty input matches nothing"
    );
}

#[test]
fn best_lets_a_whole_name_outrank_an_acronym() {
    // `Wx` is an application literally called that; 微信 only *spells* wx.
    // Typing a name must pick the name, not the acronym that resembles it.
    let by_name = entry("wx", &["Wx"], "/wx", &[]);
    let by_pinyin = entry("wechat", &["微信"], "/wechat", &[]);
    let entries = vec![by_name, by_pinyin];
    assert_eq!(score(&entries[0], "wx"), 5);
    assert_eq!(score(&entries[1], "wx"), 4);
    let top = best(&entries, "wx");
    assert_eq!(top.len(), 1);
    assert_eq!(top[0].id, "wx");
}

#[test]
fn best_keeps_ties_from_different_key_sources_in_one_group() {
    // Both entries carry wx as a word; neither is *called* wx. Nothing tells
    // them apart, so the caller has to report the ambiguity.
    let one = entry("one", &["Wx One"], "/one", &[]);
    let two = entry("two", &["Wx Two"], "/two", &[]);
    let entries = vec![one, two];
    let top = best(&entries, "wx");
    assert_eq!(top.len(), 2, "both answer wx exactly; the caller decides");
}
