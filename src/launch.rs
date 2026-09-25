//! Launching an application by name (`@native app {input}`).
//!
//! The `app` alias is a native backend like `cd`: no shell. What it adds is
//! name resolution -- the input is matched against the applications installed
//! on this machine, so `app xx` reaches the same program the desktop menu
//! does. Resolution and spawning live in [`crate::launch::desktop`] and this
//! module respectively.
//!
//! [`pinyin`] supplies initials acronyms, so a Chinese name is reachable
//! through its pinyin shorthand (`app wjglq`).

pub(crate) mod pinyin;
mod pinyin_table;
