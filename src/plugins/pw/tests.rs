//! Password-generator tests, split out so `pw.rs` stays within the size
//! budget. Deterministic except for `generate`/`run` (real `/dev/urandom`).

use super::*;

#[test]
fn marker_matches_exactly_and_tolerates_padding() {
    assert!(is_native(TEMPLATE));
    assert!(is_native(" @native pw "));
    assert!(!is_native("@native pww"));
    assert!(!is_native("@native clipboard"));
    assert!(!is_native(""));
}

#[test]
fn default_def_seeds_the_alias_with_profile_shortcuts() {
    let def = default_def();
    assert_eq!(def.name, "pw");
    assert!(def.triggers.is_empty());
    assert_eq!(def.linux.as_deref(), Some(TEMPLATE));
    assert_eq!(def.macos.as_deref(), Some(TEMPLATE));
    let keys: Vec<&str> = def.shortcuts.keys().map(String::as_str).collect();
    assert_eq!(keys, vec!["c", "m", "s"]);
    assert_eq!(def.shortcuts["s"], "simple");
    assert_eq!(def.shortcuts["m"], "medium");
    assert_eq!(def.shortcuts["c"], "complex");
}

#[test]
fn parse_empty_spec_is_complex_32() {
    assert_eq!(
        parse(""),
        Ok(Request {
            profile: COMPLEX,
            len: 32
        })
    );
    assert_eq!(
        parse("   "),
        Ok(Request {
            profile: COMPLEX,
            len: 32
        })
    );
}

#[test]
fn parse_profiles_and_their_default_lengths() {
    assert_eq!(
        parse("simple"),
        Ok(Request {
            profile: SIMPLE,
            len: 8
        })
    );
    assert_eq!(
        parse("medium"),
        Ok(Request {
            profile: MEDIUM,
            len: 16
        })
    );
    assert_eq!(
        parse("complex"),
        Ok(Request {
            profile: COMPLEX,
            len: 32
        })
    );
}

#[test]
fn parse_length_override_and_bare_number() {
    assert_eq!(
        parse("complex 24"),
        Ok(Request {
            profile: COMPLEX,
            len: 24
        })
    );
    assert_eq!(
        parse("24"),
        Ok(Request {
            profile: COMPLEX,
            len: 24
        })
    );
    // A number before the profile still validates against the final one.
    assert_eq!(
        parse("16 medium"),
        Ok(Request {
            profile: MEDIUM,
            len: 16
        })
    );
}

#[test]
fn parse_profile_names_are_case_insensitive_and_last_wins() {
    assert_eq!(
        parse("MEDIUM 12"),
        Ok(Request {
            profile: MEDIUM,
            len: 12
        })
    );
    assert_eq!(
        parse("simple medium"),
        Ok(Request {
            profile: MEDIUM,
            len: 16
        })
    );
}

#[test]
fn parse_unknown_token_names_the_usage() {
    assert_eq!(
        parse("xyz").unwrap_err(),
        "unknown pw spec: \"xyz\" (usage: [simple|medium|complex] [length])"
    );
    assert_eq!(
        parse("medium -5").unwrap_err(),
        "unknown pw spec: \"-5\" (usage: [simple|medium|complex] [length])"
    );
}

#[test]
fn parse_rejects_lengths_outside_the_bounds() {
    assert_eq!(
        parse("complex 3").unwrap_err(),
        "length 3 below minimum 4 for complex"
    );
    assert_eq!(
        parse("simple 1").unwrap_err(),
        "length 1 below minimum 2 for simple"
    );
    assert_eq!(parse("130").unwrap_err(), "length 130 above maximum 128");
    // Numbers too large even for usize parse into the same error.
    assert_eq!(
        parse("99999999999999999999999999").unwrap_err(),
        "length 99999999999999999999999999 above maximum 128"
    );
}

#[test]
fn below_is_uniform_and_rejects_above_the_limit() {
    let mut pos = 0;
    assert_eq!(below(&[0, 1, 2, 3], &mut pos, 2), Some(0));
    assert_eq!(below(&[0, 1, 2, 3], &mut pos, 2), Some(1));
    // 255 is at/above limit 255 for k=3: skipped, the next byte decides.
    let mut pos = 0;
    assert_eq!(below(&[255, 254], &mut pos, 3), Some(254 % 3));
    // Exhausted stream.
    let mut pos = 4;
    assert_eq!(below(&[0, 0, 0, 0], &mut pos, 2), None);
    assert_eq!(below(&[], &mut 0, 26), None);
}

/// Zero entropy indexes 0 everywhere: one first-char per active set, the
/// rest filled with the alphabet head, then the always-swap-with-0 shuffle.
/// The exact multiset proves no character is lost or invented.
#[test]
fn build_with_zero_entropy_keeps_every_char() {
    let req = parse("complex 32").unwrap();
    let out = build(&req, &[0u8; 512]).unwrap();
    assert_eq!(out.len(), 32);
    let alphabet: Vec<u8> = req
        .profile
        .sets
        .iter()
        .flat_map(|s| s.iter().copied())
        .collect();
    let mut expected: Vec<u8> = req.profile.sets.iter().map(|s| s[0]).collect();
    expected.extend(std::iter::repeat_n(
        alphabet[0],
        req.len - req.profile.sets.len(),
    ));
    let mut out_sorted: Vec<u8> = out.bytes().collect();
    out_sorted.sort_unstable();
    let mut expected_sorted = expected;
    expected_sorted.sort_unstable();
    assert_eq!(out_sorted, expected_sorted, "no char lost or invented");
    // Every active set is represented (all-zero entropy places each set
    // head before the shuffle).
    for set in req.profile.sets {
        assert!(out.contains(set[0] as char), "missing a char from a set");
    }
}

#[test]
fn build_medium_has_no_specials_and_simple_no_uppercase() {
    let medium = build(&parse("medium").unwrap(), &[0u8; 512]).unwrap();
    assert_eq!(medium.len(), 16);
    assert!(!medium.bytes().any(|b| SPECIAL.contains(&b)));
    let simple = build(&parse("simple").unwrap(), &[0u8; 512]).unwrap();
    assert_eq!(simple.len(), 8);
    assert!(!simple.bytes().any(|b| UPPER.contains(&b)));
}

#[test]
fn build_rechecks_the_minimum_and_needs_entropy() {
    let too_short = Request {
        profile: COMPLEX,
        len: 3,
    };
    assert_eq!(
        build(&too_short, &[0u8; 512]).unwrap_err(),
        "length 3 below minimum 4 for complex"
    );
    let req = parse("complex").unwrap();
    assert_eq!(
        build(&req, &[]).unwrap_err(),
        "pw: entropy exhausted after 0 bytes"
    );
    // Enough bytes for the placements but not the shuffle.
    assert!(build(&req, &[0u8; 40]).is_err());
}

#[test]
fn generate_produces_distinct_passwords_of_the_right_shape() {
    let req = parse("medium 16").unwrap();
    let a = generate(&req).unwrap();
    let b = generate(&req).unwrap();
    for out in [&a, &b] {
        assert_eq!(out.len(), 16);
        assert!(out.bytes().any(|b| LOWER.contains(&b)));
        assert!(out.bytes().any(|b| UPPER.contains(&b)));
        assert!(out.bytes().any(|b| DIGIT.contains(&b)));
    }
    assert_ne!(a, b, "two draws from /dev/urandom collide");
}

#[test]
fn run_reports_the_password_and_survives_a_dead_clipboard() {
    // A bad spec fails before anything is generated.
    assert_eq!(
        run("nope"),
        ExecOutcome::Failure(
            "unknown pw spec: \"nope\" (usage: [simple|medium|complex] [length])".to_string()
        )
    );
    // Generation itself works headless: the message either reports the
    // copy or says it failed, but the password is always present.
    match run("") {
        ExecOutcome::Success(msg) => {
            assert!(msg.starts_with("pw ok: "), "{msg}");
            assert!(
                msg.ends_with("(copied)") || msg.contains("(copy failed: "),
                "{msg}"
            );
            let password = msg
                .strip_prefix("pw ok: ")
                .and_then(|rest| rest.split(" (").next())
                .unwrap();
            assert_eq!(password.len(), 32);
        }
        other => panic!("expected Success, got {other:?}"),
    }
}
