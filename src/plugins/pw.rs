//! Native password generator backing the seeded `pw` alias.
//!
//! Profiles: `simple` (lowercase + digits, 8), `medium` (upper + lower +
//! digits, 16) and `complex` (those + specials, 32 - the
//! default). The spec is `[simple|medium|complex] [length]`, so `pw`,
//! `pw medium` and `pw c 24` (the `c` shortcut expands to `complex`) all
//! work; a trailing number overrides the profile's default length.
//!
//! Entropy comes from `/dev/urandom` (no `rand` dependency) and is mapped
//! to charset indices by rejection sampling, so every character is
//! uniformly distributed. The generated password is copied to the
//! clipboard and echoed in the status line; it is never persisted -
//! history records only the spec (`pw medium 16`), which stays replayable.

use crate::alias::AliasDef;
use crate::exec::ExecOutcome;
use std::collections::BTreeMap;

/// Marker template handled by this module instead of the shell.
pub const TEMPLATE: &str = "@native pw";

/// Longest password accepted; keeps the bar's status line sane.
pub const MAX_LEN: usize = 128;

const LOWER: &[u8] = b"abcdefghijklmnopqrstuvwxyz";
const UPPER: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ";
const DIGIT: &[u8] = b"0123456789";
/// Specials restricted to quote-safe characters (no quotes, spaces,
/// backslashes or backticks), so the password survives paste-into-CLI use.
/// `!` still expands under interactive bash history, so quote passwords
/// pasted there.
const SPECIAL: &[u8] = b"!@#$%^&*-_=+?~";

/// A generation profile: which charsets are active and the default length.
/// Plain data - a name, the active sets and the length an unspecified spec
/// gets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Profile {
    pub name: &'static str,
    pub sets: &'static [&'static [u8]],
    pub default_len: usize,
}

/// Lowercase letters and digits only.
pub const SIMPLE: Profile = Profile {
    name: "simple",
    sets: &[LOWER, DIGIT],
    default_len: 8,
};

/// Letters (both cases) and digits.
pub const MEDIUM: Profile = Profile {
    name: "medium",
    sets: &[LOWER, UPPER, DIGIT],
    default_len: 16,
};

/// Letters, digits and specials; the profile an empty spec selects.
pub const COMPLEX: Profile = Profile {
    name: "complex",
    sets: &[LOWER, UPPER, DIGIT, SPECIAL],
    default_len: 32,
};

/// The profile whose name matches `token`, ASCII case-insensitively.
fn profile_by_name(token: &str) -> Option<Profile> {
    [SIMPLE, MEDIUM, COMPLEX]
        .into_iter()
        .find(|p| p.name.eq_ignore_ascii_case(token))
}

/// One generation request: the profile plus the final length.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Request {
    pub profile: Profile,
    pub len: usize,
}

/// Parse a `pw` spec: whitespace-split tokens, profile names ASCII
/// case-insensitive (last one wins), one numeric length override. An empty
/// spec means the `complex` profile at its default length.
///
/// Errors carry a one-line usage hint:
/// * unknown token - `unknown pw spec: "xyz" (usage: ...)`
/// * length below the active set count - one guaranteed char per set must fit
/// * length above [`MAX_LEN`] - also the fate of unparsable numbers such as
///   `99999999999999999999999999`
pub fn parse(spec: &str) -> Result<Request, String> {
    let mut profile = COMPLEX;
    let mut len: Option<u128> = None;
    for token in spec.split_whitespace() {
        if let Some(found) = profile_by_name(token) {
            profile = found;
            continue;
        }
        match token.parse::<u128>() {
            Ok(n) if n > MAX_LEN as u128 => {
                return Err(format!("length {n} above maximum {MAX_LEN}"))
            }
            Ok(n) => len = Some(n),
            Err(_) => {
                return Err(format!(
                    "unknown pw spec: \"{token}\" (usage: [simple|medium|complex] [length])"
                ))
            }
        }
    }
    let len = len.unwrap_or(profile.default_len as u128);
    if len < profile.sets.len() as u128 {
        return Err(format!(
            "length {len} below minimum {} for {}",
            profile.sets.len(),
            profile.name
        ));
    }
    Ok(Request {
        profile,
        len: len as usize,
    })
}

/// Unbiased uniform index in `0..k` via rejection sampling over the byte
/// stream: bytes at or above `limit` (the largest multiple of `k` that fits
/// a byte) are discarded so the modulo never skews the distribution.
/// `k <= 256` here; `None` when the entropy is exhausted.
fn below(entropy: &[u8], pos: &mut usize, k: usize) -> Option<usize> {
    if k == 0 || k > 256 {
        return None;
    }
    let limit = 256 - (256 % k);
    loop {
        let byte = *entropy.get(*pos)? as usize;
        *pos += 1;
        if byte < limit {
            return Some(byte % k);
        }
    }
}

/// Deterministic given `entropy`: place one character from each active set
/// (so every set is represented), fill the rest from the combined alphabet,
/// then Fisher-Yates shuffle with [`below`]-driven swaps. `Err` when
/// `req.len` is below the set count (re-checked) or the entropy runs out.
pub fn build(req: &Request, entropy: &[u8]) -> Result<String, String> {
    if req.len < req.profile.sets.len() {
        return Err(format!(
            "length {} below minimum {} for {}",
            req.len,
            req.profile.sets.len(),
            req.profile.name
        ));
    }
    let mut pos = 0usize;
    let mut chars: Vec<u8> = Vec::with_capacity(req.len);
    let exhausted = |pos: usize| format!("pw: entropy exhausted after {pos} bytes");
    for set in req.profile.sets {
        let idx = below(entropy, &mut pos, set.len()).ok_or_else(|| exhausted(pos))?;
        chars.push(set[idx]);
    }
    let alphabet: Vec<u8> = req
        .profile
        .sets
        .iter()
        .flat_map(|s| s.iter().copied())
        .collect();
    while chars.len() < req.len {
        let idx = below(entropy, &mut pos, alphabet.len()).ok_or_else(|| exhausted(pos))?;
        chars.push(alphabet[idx]);
    }
    for i in (1..chars.len()).rev() {
        let j = below(entropy, &mut pos, i + 1).ok_or_else(|| exhausted(pos))?;
        chars.swap(i, j);
    }
    Ok(chars.into_iter().map(|b| b as char).collect())
}

/// Read `n` bytes from `/dev/urandom` (Linux and macOS both provide it).
fn urandom(n: usize) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    let mut buf = vec![0u8; n];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut buf)?;
    Ok(buf)
}

/// Generate one password for `req`: draw `len * 16 + 64` entropy bytes
/// (rejection sampling consumes more than one byte per choice) and hand
/// them to the pure [`build`].
pub fn generate(req: &Request) -> Result<String, String> {
    let entropy =
        urandom(req.len * 16 + 64).map_err(|e| format!("pw: reading /dev/urandom failed: {e}"))?;
    build(req, &entropy)
}

/// True when an alias template selects this native backend.
pub fn is_native(template: &str) -> bool {
    template.trim() == TEMPLATE
}

/// The seeded `pw` alias: this backend on both platforms, no triggers, with
/// the three profiles bound to single-letter shortcuts (`pw s`, `pw m`,
/// `pw c`).
pub fn default_def() -> AliasDef {
    AliasDef {
        name: "pw".to_string(),
        triggers: vec![],
        linux: Some(TEMPLATE.to_string()),
        macos: Some(TEMPLATE.to_string()),
        shortcuts: BTreeMap::from([
            ("s".to_string(), "simple".to_string()),
            ("m".to_string(), "medium".to_string()),
            ("c".to_string(), "complex".to_string()),
        ]),
    }
}

/// Runtime entry: parse and generate; a bad spec or failed generation is a
/// `Failure`. The password itself lives only in the success message and
/// the clipboard - a failed clipboard copy still succeeds (generation
/// worked), the message just says so.
pub fn run(input: &str) -> ExecOutcome {
    let req = match parse(input) {
        Ok(req) => req,
        Err(msg) => return ExecOutcome::Failure(msg),
    };
    let password = match generate(&req) {
        Ok(password) => password,
        Err(msg) => return ExecOutcome::Failure(msg),
    };
    match crate::clipboard::copy(&password) {
        Ok(()) => ExecOutcome::Success(format!("pw ok: {password} (copied)")),
        Err(err) => ExecOutcome::Success(format!("pw ok: {password} (copy failed: {err})")),
    }
}

#[cfg(test)]
#[path = "pw/tests.rs"]
mod tests;
