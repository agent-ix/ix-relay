//! The hook's stdin, read inside the hook's deadline (NFR-001-M-5).
//!
//! A harness may never close, or may stall, the pipe it feeds a hook. Reading to
//! end of file would then hold the hook outside any time budget, so the real
//! reader ([`ThreadedStdin`]) reads on a helper thread and the caller waits on it
//! with the remaining [`Budget`]. The wait ends at the first of: the end of the
//! first JSON object (the harness need not close the pipe, see [`ObjectScan`]),
//! end of file, a read error, [`MAX_INPUT_BYTES`], an idle stall of
//! [`IDLE_WINDOW`], or the budget. The idle window restarts whenever bytes
//! arrive, so a large payload written slowly is not a timeout while it keeps
//! progressing; the budget still caps the whole read.
//!
//! The helper thread is detached. On a timeout it stays blocked in `read`, and
//! the process exits right after, which ends it; nothing joins it.

use std::{
    io::{self, Read},
    sync::mpsc::{self, RecvTimeoutError},
    thread,
    time::Duration,
};

use crate::budget::Budget;

/// The most stdin bytes a hook accepts: 8 MiB, the repo's bound for untrusted
/// local reads (`MAX_TRANSCRIPT_SCAN_BYTES`). It is not "a few hundred bytes":
/// a `PostToolUse` payload carries the tool's input and response, so an Edit or
/// Write of a large file or a Bash with long output is megabytes. Past the cap
/// the call is a recorded `too_large`, never an unbounded allocation.
pub(crate) const MAX_INPUT_BYTES: usize = 8 << 20;

/// Size of one read on the helper thread.
const CHUNK_BYTES: usize = 64 * 1024;

/// How long stdin may deliver nothing before the read gives up (NFR-001-M-5).
/// It restarts at every chunk of bytes; the budget passed to the read bounds
/// the total.
pub(crate) const IDLE_WINDOW: Duration = Duration::from_millis(300);

/// How often the wait re-reads the budget's clock.
const POLL: Duration = Duration::from_millis(10);

/// Why stdin did not yield a payload.
#[derive(Debug)]
pub(crate) enum InputFault {
    /// Reading (or starting the reader) failed.
    Io(io::Error),
    /// More than [`MAX_INPUT_BYTES`] arrived.
    TooLarge,
    /// No bytes arrived for [`IDLE_WINDOW`], or the budget ran out, before the
    /// object ended or the pipe closed.
    Timeout,
}

/// Where a hook's input bytes come from.
pub(crate) trait InputSource {
    /// Reads the input, giving up when `budget` is spent. A source that can stall
    /// returns at the end of the first JSON object instead of waiting for end of
    /// file.
    fn read_input(&mut self, budget: &Budget<'_>) -> Result<Vec<u8>, InputFault>;
}

/// A plain reader is read to end of file, capped. It has no deadline, so it is
/// only for in-memory sources in tests: compiling it into the binary would let
/// `io::stdin().lock()` stand in for [`ThreadedStdin`] again.
#[cfg(test)]
impl<R: Read> InputSource for R {
    fn read_input(&mut self, _budget: &Budget<'_>) -> Result<Vec<u8>, InputFault> {
        let cap = u64::try_from(MAX_INPUT_BYTES).unwrap_or(u64::MAX);
        let mut buf = Vec::new();
        self.by_ref()
            .take(cap.saturating_add(1))
            .read_to_end(&mut buf)
            .map_err(InputFault::Io)?;
        if buf.len() > MAX_INPUT_BYTES {
            return Err(InputFault::TooLarge);
        }
        Ok(buf)
    }
}

/// Finds where the first JSON object ends, one byte at a time, so the cost of
/// the whole read is linear in the bytes read and a chunk is never re-scanned.
///
/// It tracks only brace depth, whether the scan is inside a string, and whether
/// the last byte was a backslash, which is all that locates the closing brace
/// (`"`, `\`, `{` and `}` are ASCII and never occur inside a multi-byte UTF-8
/// sequence). It does not validate: the object is decoded afterwards. Anything
/// that cannot begin an object (the first non-whitespace byte is not `{`) also
/// ends the scan at once, so `[]`, `not json` or a bare value on a held-open
/// pipe is a `malformed` decode straight away, not a wait for the deadline.
#[derive(Debug, Default)]
pub(crate) struct ObjectScan {
    depth: usize,
    started: bool,
    in_string: bool,
    escaped: bool,
}

impl ObjectScan {
    /// Feeds `bytes`; true once the scan has an answer (the object closed, or
    /// the input cannot be an object). Bytes after that are not examined.
    pub(crate) fn feed(&mut self, bytes: &[u8]) -> bool {
        for &b in bytes {
            if !self.started {
                match b {
                    b' ' | b'\t' | b'\r' | b'\n' => continue,
                    b'{' => {
                        self.started = true;
                        self.depth = 1;
                        continue;
                    }
                    _ => return true,
                }
            }
            if self.in_string {
                match (self.escaped, b) {
                    (true, _) => self.escaped = false,
                    (false, b'\\') => self.escaped = true,
                    (false, b'"') => self.in_string = false,
                    (false, _) => {}
                }
                continue;
            }
            match b {
                b'"' => self.in_string = true,
                b'{' | b'[' => self.depth = self.depth.saturating_add(1),
                b'}' | b']' => {
                    self.depth = self.depth.saturating_sub(1);
                    if self.depth == 0 {
                        return true;
                    }
                }
                _ => {}
            }
        }
        false
    }
}

/// A blocking reader moved onto a helper thread on first use, so the caller can
/// wait on it with a deadline. It is one-shot: a second read finds no reader.
pub(crate) struct ThreadedStdin<R> {
    reader: Option<R>,
}

impl<R: Read + Send + 'static> ThreadedStdin<R> {
    /// Wraps `reader`; no thread starts until [`InputSource::read_input`].
    pub(crate) fn new(reader: R) -> Self {
        Self {
            reader: Some(reader),
        }
    }
}

impl<R: Read + Send + 'static> InputSource for ThreadedStdin<R> {
    fn read_input(&mut self, budget: &Budget<'_>) -> Result<Vec<u8>, InputFault> {
        let Some(mut reader) = self.reader.take() else {
            return Err(InputFault::Io(io::Error::other("stdin was already read")));
        };
        // Bounded, so a fast writer waits for the consumer instead of piling up.
        let (tx, rx) = mpsc::sync_channel::<io::Result<Vec<u8>>>(4);
        // Detached on purpose (see the module docs).
        thread::Builder::new()
            .name("hook-stdin".to_owned())
            .spawn(move || {
                let mut chunk = vec![0u8; CHUNK_BYTES];
                loop {
                    let sent = match reader.read(&mut chunk) {
                        Ok(n) => tx.send(Ok(chunk[..n].to_vec())).is_ok() && n > 0,
                        Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                        Err(e) => {
                            let _ = tx.send(Err(e));
                            false
                        }
                    };
                    if !sent {
                        return;
                    }
                }
            })
            .map_err(InputFault::Io)?;

        collect(budget, |wait| match rx.recv_timeout(wait) {
            Ok(chunk) => Wake::Chunk(chunk),
            Err(RecvTimeoutError::Timeout) => Wake::Quiet,
            Err(RecvTimeoutError::Disconnected) => Wake::Closed,
        })
    }
}

/// What one wait on the reader thread produced.
pub(crate) enum Wake {
    /// A read finished: bytes (empty is end of file) or an error.
    Chunk(io::Result<Vec<u8>>),
    /// Nothing arrived within the wait.
    Quiet,
    /// The reader thread is gone.
    Closed,
}

/// Collects the first object from `wait`, which blocks for at most the duration
/// it is given. This is the deadline logic, apart from the thread, so a test can
/// script arrivals against a fake clock.
pub(crate) fn collect(
    budget: &Budget<'_>,
    mut wait: impl FnMut(Duration) -> Wake,
) -> Result<Vec<u8>, InputFault> {
    let mut buf = Vec::new();
    let mut scan = ObjectScan::default();
    let mut last_progress = budget.now();
    loop {
        let now = budget.now();
        let left = budget
            .left_within_idle(now, last_progress, IDLE_WINDOW)
            .ok_or(InputFault::Timeout)?;
        match wait(left.min(POLL)) {
            // A zero-length chunk is end of file.
            Wake::Chunk(Ok(chunk)) if chunk.is_empty() => return Ok(buf),
            Wake::Chunk(Ok(chunk)) => {
                if buf.len().saturating_add(chunk.len()) > MAX_INPUT_BYTES {
                    return Err(InputFault::TooLarge);
                }
                // Stamped when the bytes arrived, not before the wait began, so
                // the window is a full 300 ms of silence.
                last_progress = budget.now();
                buf.extend_from_slice(&chunk);
                if scan.feed(&chunk) {
                    return Ok(buf);
                }
            }
            Wake::Chunk(Err(e)) => return Err(InputFault::Io(e)),
            Wake::Quiet => {}
            Wake::Closed => return Ok(buf),
        }
    }
}

/// Arrival scripts for tests that read through [`collect`] on a fake clock.
#[cfg(test)]
pub(crate) mod testing {
    use std::time::Duration;

    use super::Wake;
    use crate::budget::testing::FakeClock;

    /// A `wait` for [`super::collect`] serving `arrivals` (absolute
    /// milliseconds on `clock`, in order): a wait returns the next chunk when it
    /// is due within the wait, moving the clock to its arrival, and otherwise moves
    /// the clock by exactly the wait, so a wait never spends more than it was given.
    pub(crate) fn arrivals<'a>(
        clock: &'a FakeClock,
        arrivals: &'a [(u64, &'static [u8])],
    ) -> impl FnMut(Duration) -> Wake + 'a {
        let mut next = arrivals.iter();
        let mut due = next.next();
        move |wait| {
            let now = crate::budget::Clock::elapsed(clock);
            match due {
                Some(&(at, bytes)) if Duration::from_millis(at) <= now + wait => {
                    clock.advance(Duration::from_millis(at).saturating_sub(now));
                    due = next.next();
                    Wake::Chunk(Ok(bytes.to_vec()))
                }
                _ => {
                    clock.advance(wait);
                    Wake::Quiet
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::Receiver;

    use proptest::prelude::*;

    use super::*;
    use crate::{
        budget::{
            Clock,
            testing::{FakeClock, TickingClock},
        },
        hook::HookInput,
    };

    /// A reader that yields what the test sends, one send per read (each at most
    /// [`CHUNK_BYTES`]), and blocks between sends: a pipe whose writer is slow or
    /// gone quiet. Dropping the sender is end of file.
    struct PipeReader(Receiver<Vec<u8>>);

    impl Read for PipeReader {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let Ok(bytes) = self.0.recv() else {
                return Ok(0);
            };
            assert!(bytes.len() <= buf.len(), "a send larger than one read");
            buf[..bytes.len()].copy_from_slice(&bytes);
            Ok(bytes.len())
        }
    }

    fn pipe() -> (mpsc::Sender<Vec<u8>>, ThreadedStdin<PipeReader>) {
        let (tx, rx) = mpsc::channel();
        (tx, ThreadedStdin::new(PipeReader(rx)))
    }

    /// Reads `stdin` on a clock that moves 10 ms per poll against a 3 s budget: a
    /// regression that waits forever ends as a `Timeout` in about that much real
    /// time instead of hanging the suite.
    fn read(stdin: &mut ThreadedStdin<PipeReader>) -> Result<Vec<u8>, InputFault> {
        let clock = TickingClock::new(Duration::from_millis(10));
        let budget = Budget::new(&clock, Duration::from_secs(3));
        stdin.read_input(&budget)
    }

    /// Tracing: TC-N001-M-05
    /// ACs: NFR-001-M-5
    #[test]
    fn tc_n001_m_05_a_silent_pipe_times_out_on_the_injected_clock() {
        // Each clock reading is 100 ms, so a 300 ms budget is spent in a few polls.
        let clock = TickingClock::new(Duration::from_millis(100));
        let budget = Budget::new(&clock, Duration::from_millis(300));
        let (_keep_open, mut stdin) = pipe();
        let fault = stdin.read_input(&budget).unwrap_err();
        assert!(matches!(fault, InputFault::Timeout), "{fault:?}");
    }

    /// Runs [`collect`] on a fake clock against arrivals at absolute times, as a
    /// real pipe behaves: a wait of `w` returns the next chunk if it is due within
    /// `w` (the clock moves to its arrival), and otherwise moves the clock by
    /// exactly `w`. Returns the outcome and the clock reading when the read ended.
    fn scripted(
        total: Duration,
        arrivals: &[(u64, &'static [u8])],
    ) -> (Result<Vec<u8>, InputFault>, Duration) {
        let clock = FakeClock::default();
        let budget = Budget::new(&clock, total);
        let out = collect(&budget, testing::arrivals(&clock, arrivals));
        (out, clock.elapsed())
    }

    const MS: fn(u64) -> Duration = Duration::from_millis;

    /// Tracing: TC-N001-M-05
    /// ACs: NFR-001-M-5
    #[test]
    fn tc_n001_m_05_input_that_keeps_arriving_outlasts_one_window() {
        // Four pieces 200 ms apart: 800 ms in all, never 300 ms without bytes.
        let (got, at) = scripted(
            MS(900),
            &[
                (200, b"{\"a\":"),
                (400, b"[1,"),
                (600, b"2],\"b\""),
                (800, b":1}"),
            ],
        );
        assert_eq!(got.unwrap(), br#"{"a":[1,2],"b":1}"#);
        assert_eq!(at, MS(800), "ends when the object does");
    }

    /// Tracing: TC-N001-M-05
    /// ACs: NFR-001-M-5
    #[test]
    fn tc_n001_m_05_the_idle_window_is_300_ms_of_silence_after_the_last_bytes() {
        // Silence after progress ends 300 ms after the last bytes, not before.
        let (got, at) = scripted(MS(900), &[(250, b"{\"a\":")]);
        assert!(matches!(got, Err(InputFault::Timeout)), "{got:?}");
        assert_eq!(at, MS(550));
        // Nothing at all: 300 ms from the start.
        let (got, at) = scripted(MS(900), &[]);
        assert!(matches!(got, Err(InputFault::Timeout)), "{got:?}");
        assert_eq!(at, MS(300));
        // A gap of 299 ms is not a stall (the window is not shortened by the
        // poll that was in flight when the bytes arrived); 301 ms is.
        let (got, _) = scripted(MS(900), &[(250, b"{\"a\":"), (549, b"1}")]);
        assert_eq!(got.unwrap(), br#"{"a":1}"#);
        let (got, at) = scripted(MS(900), &[(250, b"{\"a\":"), (551, b"1}")]);
        assert!(matches!(got, Err(InputFault::Timeout)), "{got:?}");
        assert_eq!(at, MS(550));
    }

    /// Tracing: TC-N001-M-05
    /// ACs: NFR-001-M-5
    #[test]
    fn tc_n001_m_05_the_overall_budget_ends_input_that_keeps_arriving_exactly_at_the_cap() {
        // A space every 200 ms never idles the window; the budget ends the read at
        // exactly 900 ms, not at the last arrival (800) or a window later.
        let drip: Vec<(u64, &[u8])> = (1..=30).map(|i| (i * 200, &b" "[..])).collect();
        let (got, at) = scripted(MS(900), &drip);
        assert!(matches!(got, Err(InputFault::Timeout)), "{got:?}");
        assert_eq!(at, MS(900));
    }

    /// Tracing: TC-N001-M-05
    /// ACs: NFR-001-M-5
    #[test]
    fn tc_n001_m_05_a_slow_payload_over_the_cap_is_still_too_large() {
        let block = vec![b' '; CHUNK_BYTES];
        let clock = FakeClock::default();
        let budget = Budget::new(&clock, Duration::from_secs(3600));
        let fault = collect(&budget, |_| {
            clock.advance(MS(200));
            Wake::Chunk(Ok(block.clone()))
        })
        .unwrap_err();
        assert!(matches!(fault, InputFault::TooLarge), "{fault:?}");
    }

    #[test]
    fn a_whole_object_returns_without_end_of_file() {
        let (tx, mut stdin) = pipe();
        tx.send(br#"{"a":1}"#.to_vec()).unwrap();
        // `tx` stays alive: the pipe is open, and the read still returns.
        assert_eq!(read(&mut stdin).unwrap(), br#"{"a":1}"#);
    }

    /// The read ends at the first thing that cannot become an object, so a held
    /// pipe with `[]` or `not json` is a decode failure at once (`malformed`), not
    /// a wait for the deadline. Complete JSON that is not a hook payload is the
    /// same: it is decoded, and refused, immediately.
    #[test]
    fn input_that_cannot_be_an_object_ends_the_read_at_once() {
        for input in [&b"[]"[..], b"not json", b"7", b"  \n[1]"] {
            let (tx, mut stdin) = pipe();
            tx.send(input.to_vec()).unwrap();
            assert_eq!(read(&mut stdin).unwrap(), input, "{input:?}");
        }
    }

    #[test]
    fn a_payload_in_pieces_is_joined_and_end_of_file_ends_a_partial_one() {
        let (tx, mut stdin) = pipe();
        tx.send(b"{\"a\"".to_vec()).unwrap();
        tx.send(b":1}".to_vec()).unwrap();
        assert_eq!(read(&mut stdin).unwrap(), br#"{"a":1}"#);
        let (tx, mut stdin) = pipe();
        tx.send(b"{\"a\":".to_vec()).unwrap();
        drop(tx);
        assert_eq!(read(&mut stdin).unwrap(), br#"{"a":"#);
    }

    /// Tracing: TC-030-01
    /// ACs: FR-030-AC-1
    #[test]
    fn more_than_the_cap_is_too_large_and_exactly_the_cap_is_not() {
        let block = vec![b' '; CHUNK_BYTES];
        let (tx, mut stdin) = pipe();
        for _ in 0..=(MAX_INPUT_BYTES / CHUNK_BYTES) {
            tx.send(block.clone()).unwrap();
        }
        let fault = read(&mut stdin).unwrap_err();
        assert!(matches!(fault, InputFault::TooLarge), "{fault:?}");

        let (tx, mut stdin) = pipe();
        for _ in 0..(MAX_INPUT_BYTES / CHUNK_BYTES) {
            tx.send(block.clone()).unwrap();
        }
        drop(tx);
        assert_eq!(read(&mut stdin).unwrap().len(), MAX_INPUT_BYTES);
    }

    #[test]
    fn a_plain_reader_is_capped_too() {
        let clock = TickingClock::new(Duration::from_millis(1));
        let budget = Budget::new(&clock, Duration::from_secs(3));
        let mut big = io::repeat(b' ').take(u64::try_from(MAX_INPUT_BYTES).unwrap() + 10);
        let fault = big.read_input(&budget).unwrap_err();
        assert!(matches!(fault, InputFault::TooLarge), "{fault:?}");
    }

    /// A payload that exercises every scanner state: nested braces and brackets,
    /// braces inside strings, escaped quotes and backslashes, multi-byte UTF-8.
    const NASTY: &str = concat!(
        r#"{"hook_event_name":"Stop","session_id":"s{[\"}]\\","#,
        r#""cwd":"/wé\"}","tool_response":{"a":[{"b":"}]é😀"},[]],"c":{}}}"#
    );

    /// Serves `whole` in pieces cut at `cuts`, the pipe held open, and returns
    /// what the read returned.
    fn read_split(whole: &[u8], cuts: &[usize]) -> Vec<u8> {
        let (tx, mut stdin) = pipe();
        let mut at = 0;
        for &cut in cuts.iter().chain(&[whole.len()]) {
            let cut = cut.clamp(at, whole.len());
            if cut > at {
                tx.send(whole[at..cut].to_vec()).unwrap();
                at = cut;
            }
        }
        read(&mut stdin).unwrap()
    }

    /// Tracing: TC-N001-M-05
    /// ACs: NFR-001-M-5
    #[test]
    fn tc_n001_m_05_a_payload_cut_at_every_byte_reads_as_the_whole_one() {
        let whole = NASTY.as_bytes();
        assert!(
            crate::hook_run::decode_input(whole).is_ok(),
            "the payload is valid"
        );
        for cut in 1..whole.len() {
            let got = read_split(whole, &[cut]);
            assert_eq!(got, whole, "cut at {cut}");
        }
    }

    proptest! {
        /// Cuts at arbitrary places of a payload with an arbitrary string inside
        /// still read as the whole payload, and decode the same.
        #[test]
        fn tc_n001_m_05_arbitrary_cuts_of_arbitrary_strings_read_as_the_whole_payload(
            text in any::<String>(),
            mut cuts in proptest::collection::vec(0usize..400, 0..6),
        ) {
            let whole = format!(
                r#"{{"hook_event_name":"Stop","session_id":{},"x":[{{"y":{{}}}}]}}"#,
                serde_json::to_string(&text).unwrap()
            );
            cuts.sort_unstable();
            let got = read_split(whole.as_bytes(), &cuts);
            prop_assert_eq!(&got, whole.as_bytes());
            let decoded: HookInput = serde_json::from_slice(&got).unwrap();
            // A text that is not a `SessionId` reads as no session (the hook never
            // fails a turn over the payload); any other text comes back whole.
            prop_assert_eq!(decoded.session_id, text.parse::<relay_core::SessionId>().ok());
        }
    }
}
