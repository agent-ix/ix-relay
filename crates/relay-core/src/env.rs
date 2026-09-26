//! This machine's name; the normalisation rule itself lives in [`MachineName`].

use crate::{error::Error, ids::MachineName};

/// This machine's name on the bus: `IX_RELAY_MACHINE` when set, else the
/// hostname, normalized (see [`MachineName`]). Used as the
/// `default` tier of [`crate::config`]'s machine-name resolution (AGE-2017) —
/// `~/.ix/relay/config.toml`'s `machine` key and the `IX_RELAY_MACHINE`
/// override still take precedence over it, same as before that file existed.
///
/// # Errors
///
/// [`Error::InvalidId`] when the value normalises to nothing usable (`.`).
pub fn machine_name() -> Result<MachineName, Error> {
    // A set-but-empty variable is unset (FR-026): a leftover `export
    // IX_RELAY_MACHINE=` must fall through to the hostname, not name the machine "".
    let raw = std::env::var("IX_RELAY_MACHINE")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| gethostname::gethostname().to_string_lossy().into_owned());
    MachineName::new(&raw)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_env::{ENV_LOCK, EnvGuard};

    #[test]
    fn uses_ix_relay_machine_when_set() {
        let _guard = ENV_LOCK.lock().unwrap();
        let _env = EnvGuard::isolated(&[("IX_RELAY_MACHINE", "Test-Machine-Override.local")]);
        assert_eq!(machine_name().unwrap().as_str(), "test-machine-override");
    }

    #[test]
    fn falls_back_to_hostname_when_unset() {
        let _guard = ENV_LOCK.lock().unwrap();
        let _env = EnvGuard::isolated(&[]);
        let expected = MachineName::new(&gethostname::gethostname().to_string_lossy()).unwrap();
        assert_eq!(machine_name().unwrap(), expected);
    }
}
