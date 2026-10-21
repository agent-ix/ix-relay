//! Compact JSON lines for the agent-facing CLI outputs.
//!
//! The payload types own their field names and order. The kit provides the
//! encoder; this module adds the one newline required by the CLI contract.

use std::io::{self, Write};

use serde::Serialize;

pub(crate) fn write_line<T: Serialize + ?Sized>(out: &mut impl Write, value: &T) -> io::Result<()> {
    ix_cli_kit::json::write_compact(out, value)?;
    out.write_all(b"\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Trace: FR-049-AC-1
    #[test]
    fn compact_line_preserves_struct_field_order_and_has_one_newline() {
        #[derive(Serialize)]
        struct Payload<'a> {
            z: &'a str,
            a: u8,
        }

        let mut output = Vec::new();
        write_line(&mut output, &Payload { z: "x", a: 2 }).unwrap();
        assert_eq!(output, b"{\"z\":\"x\",\"a\":2}\n");
    }
}
