//! Pinyin initials, for matching a name by the acronym a launcher user types.
//!
//! `文件管理器` (wen jian guan li qi) is reachable as `wjglq`, exactly the
//! shorthand someone types in a launcher. Only initials are computed: a full
//! transliteration would dwarf the feature, and initials are what a
//! one-line launcher input actually needs. Heteronyms are kept, so `音乐`
//! answers to both `yl` (yin le) and `yy` (yin yue), and `重庆` to `zq` and
//! `cq`; a pinyin reading outside [`super::pinyin_table`] adds nothing and
//! the character is simply skipped.
//!
//! ASCII input passes through unchanged, so `QQ音乐` spells `qqyl` / `qqyy`
//! and mixed names stay predictable.

use super::pinyin_table::GROUPS;

/// How many acronym variants a single name may produce before the tail is
/// dropped. Reached only by pathological names built from many heteronyms.
const MAX_VARIANTS: usize = 128;

/// Initial letters of one character: itself for ASCII letters and digits,
/// and every reading's first letter for a character in the table.
///
/// Spaces, punctuation and unknown characters yield no letters at all.
pub fn initials(character: char) -> Vec<char> {
    if character.is_ascii_alphanumeric() {
        return vec![character.to_ascii_lowercase()];
    }
    let mask = initial_mask(character);
    (0..26u32)
        .filter(|shift| mask & (1 << shift) != 0)
        .map(|shift| (b'a' + shift as u8) as char)
        .collect()
}

/// Bit `n` set when `character` has a reading starting with `a + n`.
pub fn initial_mask(character: char) -> u32 {
    let mut mask = 0;
    for (letter, characters) in GROUPS {
        if characters.contains(character) {
            mask |= 1 << (*letter as u32 - 'a' as u32);
        }
    }
    mask
}

/// Every initials acronym `name` can spell, most common readings first.
///
/// Once [`MAX_VARIANTS`] is hit the remaining combinations are dropped, so
/// an acronym is not guaranteed to be listed for absurdly ambiguous input.
/// Acronyms shorter than two letters are dropped: a one-letter key is noise
/// in a launcher, where the full name already matches.
pub fn acronyms(name: &str) -> Vec<String> {
    let mut variants = vec![String::new()];
    for character in name.chars() {
        let letters = initials(character);
        if letters.is_empty() {
            continue;
        }
        let mut grown = Vec::with_capacity(variants.len() * letters.len());
        for variant in &variants {
            for letter in &letters {
                let mut next = variant.clone();
                next.push(*letter);
                grown.push(next);
            }
        }
        grown.truncate(MAX_VARIANTS);
        variants = grown;
    }

    let mut unique: Vec<String> = Vec::new();
    for variant in variants {
        if variant.chars().count() > 1 && !unique.contains(&variant) {
            unique.push(variant);
        }
    }
    unique
}

#[cfg(test)]
mod tests {
    use super::*;

    fn has(name: &str, acronym: &str) -> bool {
        acronyms(name).iter().any(|variant| variant == acronym)
    }

    #[test]
    fn ascii_passes_through_and_punctuation_adds_nothing() {
        assert_eq!(initials('Q'), vec!['q']);
        assert_eq!(initials('7'), vec!['7']);
        assert!(initials(' ').is_empty());
        assert!(initials('★').is_empty());
        assert!(initials('€').is_empty());
    }

    #[test]
    fn known_hanzi_carry_their_reading() {
        assert!(initials('微').contains(&'w'));
        assert!(initials('迅').contains(&'x'));
        assert!(initials('安').contains(&'a')); // zero-initial syllable: an
        assert!(initials('二').contains(&'e'));
        assert!(initials('词').contains(&'c'));
    }

    #[test]
    fn acronyms_spell_pinyin_initials() {
        assert!(has("小小备忘录", "xxbwl"), "{:?}", acronyms("小小备忘录"));
        assert!(has("文件管理器", "wjglq"), "{:?}", acronyms("文件管理器"));
        assert!(has("剪贴板管理器", "jtbglq"));
        assert!(has("百度网盘", "bdwp"));
    }

    #[test]
    fn heteronyms_keep_every_reading() {
        let music = acronyms("音乐");
        assert!(music.contains(&"yl".to_string()), "{music:?}");
        assert!(music.contains(&"yy".to_string()), "{music:?}");
        let chongqing = acronyms("重庆");
        assert!(chongqing.contains(&"zq".to_string()), "{chongqing:?}");
        assert!(chongqing.contains(&"cq".to_string()), "{chongqing:?}");
    }

    #[test]
    fn traditional_names_keep_every_character() {
        // Coverage is the CJK block, not just GB2312: a zh_TW name must
        // spell its full acronym instead of losing characters and leaving
        // an unrelated two-letter key behind (顯示器 once spelled `qq`).
        assert!(has("小小備忘錄", "xxbwl"), "{:?}", acronyms("小小備忘錄"));
        assert_eq!(acronyms("顯示器"), vec!["xsq".to_string()]);
        assert_eq!(acronyms("移除式裝置與媒體"), vec!["ycszzymt".to_string()]);
    }

    #[test]
    fn spaces_split_nothing_and_ascii_mixes_in() {
        // Every character contributes, so a two-word name spells one acronym
        // across the space.
        assert!(has("小小 备忘录", "xxbwl"));
        assert!(has("QQ音乐", "qqyl"));
        assert!(has("WPS 文字", "wpswz"));
    }

    #[test]
    fn a_name_can_answer_to_several_spellings() {
        let wechat = acronyms("微信");
        assert!(wechat.contains(&"wx".to_string()), "{wechat:?}");
        assert!(wechat.len() < MAX_VARIANTS);
    }

    #[test]
    fn variants_are_bounded_and_unique() {
        let ambiguous = "乐".repeat(40);
        let variants = acronyms(&ambiguous);
        assert!(variants.len() <= MAX_VARIANTS, "{}", variants.len());
        let mut sorted = variants.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), variants.len(), "duplicates in {variants:?}");
    }

    #[test]
    fn letters_are_dropped_for_unknown_characters() {
        assert!(acronyms("★☆").is_empty());
        assert_eq!(acronyms("海"), Vec::<String>::new());
    }
}
