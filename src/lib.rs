//! bennett-circuits: the bounded circuit specialiser for the Bennett
//! virtual ISA (ISA 0.8, consumed only through the published spec in
//! `bennett-isa-spec`). See `docs/PLAN.md` for the architecture and
//! milestone scope.

pub mod amplitude;
mod borrow;
pub mod certificate;
mod checker;
pub mod classical;
pub mod coverage;
pub mod flatten;
pub mod frame;
pub mod history;
pub mod ir;
mod ir_codec;
pub mod json;
pub mod linking;
pub mod lower;
mod metadata;
pub mod object;
pub mod oracle;
pub mod placement;
pub mod proof;
pub mod qasm;
mod selectors;
pub mod sha256;
pub mod simulation;
pub mod step_check;
pub mod target;
mod translation;

/// Crate version string.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// The one error type: a loud, contextual message. Fail fast (CLAUDE rule 1):
/// unsupported input is an error naming the construct and, where a later
/// milestone owns it, `unimplemented: <row> (milestone N)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(pub String);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

impl From<String> for Error {
    fn from(s: String) -> Self {
        Error(s)
    }
}

impl From<&str> for Error {
    fn from(s: &str) -> Self {
        Error(s.to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// Build an `Err` with a formatted message.
macro_rules! bail {
    ($($t:tt)*) => {
        return Err(crate::Error(format!($($t)*)))
    };
}
pub(crate) use bail;

#[cfg(test)]
mod tests {
    #[test]
    fn version_nonempty() {
        assert!(!super::version().is_empty());
    }
}
