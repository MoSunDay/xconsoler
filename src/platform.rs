//! Platform detection helpers.

/// The platforms xconsoler distinguishes when picking alias commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Linux,
    Macos,
}

/// Detect the current platform. Anything other than macOS is treated as Linux.
pub fn current() -> Platform {
    if cfg!(target_os = "macos") {
        Platform::Macos
    } else {
        Platform::Linux
    }
}

/// Stable lowercase name used in messages and `no command configured for ...`.
pub fn name(p: Platform) -> &'static str {
    match p {
        Platform::Linux => "linux",
        Platform::Macos => "macos",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_stable() {
        assert_eq!(name(Platform::Linux), "linux");
        assert_eq!(name(Platform::Macos), "macos");
    }

    #[test]
    fn current_returns_supported_variant() {
        assert!(matches!(current(), Platform::Linux | Platform::Macos));
    }
}
