/// Why a read failed: the fixed set a log line may name, never content.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorClass {
    /// The read ran past its budget.
    Timeout,
    /// A file or call could not be read.
    Unreadable,
    /// This platform has no way to read it.
    Unsupported,
    /// What was read did not parse.
    #[cfg_attr(
        not(any(target_os = "linux", test, feature = "fixtures")),
        allow(dead_code, reason = "only the Linux readers parse text")
    )]
    Parse,
    /// The thing to read is not there.
    Unavailable,
}

impl ErrorClass {
    /// The class as logged.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            ErrorClass::Timeout => "timeout",
            ErrorClass::Unreadable => "unreadable",
            ErrorClass::Unsupported => "unsupported",
            ErrorClass::Parse => "parse",
            ErrorClass::Unavailable => "unavailable",
        }
    }
}
