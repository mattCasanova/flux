//! Absolute-row bookkeeping for the semantic stream (v0.3a).
//!
//! alacritty's grid addresses rows relative to the screen (line 0 = top
//! of the live screen, negative lines = history) and those numbers move
//! every time output scrolls. To remember *which rows* a prompt or a
//! command's output occupy we need a row identity that survives
//! scrolling: `abs = dropped + history_size + line`, where `dropped`
//! counts every line that has ever left the top of history. As long as
//! `dropped` is exact, `abs` is stable for a content row for the life
//! of the terminal — history growth raises `history_size` by exactly the
//! number of lines pushed, and each pushed line lowers `line` by one.
//!
//! `TerminalState` owns the two facts this module cannot see (history
//! size and grid lines) and reports them; this module only stores spans
//! and answers row queries.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Absolute row index — see module docs. Signed so grid-line math
/// (`abs - dropped - history`) never underflows mid-expression.
pub(crate) type AbsRow = i64;

/// One prompt → command → output cycle as seen through OSC 133.
///
/// `A` opens a span, `B` marks where the prompt ended and the command
/// echo begins, `C` marks the first output row, `D` closes it with the
/// exit code. Header rows (prompt + echo) are `[prompt_start,
/// header_end())` — bounded by `output_start`, not `prompt_end`, so a
/// transient prompt that rewrites its rows at accept-line still
/// decorates the right ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Span {
    pub prompt_start: AbsRow,
    /// Row and column where the prompt ended (OSC 133;B).
    pub prompt_end: Option<(AbsRow, usize)>,
    /// First output row (OSC 133;C).
    pub output_start: Option<AbsRow>,
    /// Row where OSC 133;D fired — one past the last output row.
    pub end: Option<AbsRow>,
    pub exit_code: Option<i32>,
    /// When execution started (OSC 133;C arrived).
    pub started_at: Option<Instant>,
    /// Wall time from C to D.
    pub duration: Option<Duration>,
}

impl Span {
    fn new(prompt_start: AbsRow) -> Self {
        Self {
            prompt_start,
            prompt_end: None,
            output_start: None,
            end: None,
            exit_code: None,
            started_at: None,
            duration: None,
        }
    }

    /// Still waiting at the prompt — no command has started.
    pub fn at_prompt(&self) -> bool {
        self.output_start.is_none() && self.end.is_none()
    }

    pub fn is_closed(&self) -> bool {
        self.end.is_some()
    }

    /// One past the last header (prompt + echo) row.
    pub fn header_end(&self) -> AbsRow {
        let end = match (self.output_start, self.end) {
            (Some(c), _) => c,
            (None, Some(d)) => d,
            (None, None) => self
                .prompt_end
                .map(|(row, _)| row + 1)
                .unwrap_or(self.prompt_start + 1),
        };
        end.max(self.prompt_start)
    }

    /// The last row this span still owns — used for pruning. `end` is
    /// one past the last output row (where `D` fired, i.e. the next
    /// prompt's first row).
    fn last_row(&self) -> AbsRow {
        (self.end.unwrap_or_else(|| self.header_end()) - 1).max(self.prompt_start)
    }
}

/// Ordered list of spans plus the `dropped` counter that anchors
/// absolute rows. See module docs.
#[derive(Debug, Default)]
pub(crate) struct SpanTracker {
    /// Lines that have left the top of history since the terminal
    /// started (including lines wiped by `CSI 3 J` / RIS).
    dropped: AbsRow,
    spans: VecDeque<Span>,
}

impl SpanTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Absolute row for a grid line given the current history size.
    pub fn abs(&self, history_size: usize, line: i32) -> AbsRow {
        self.dropped + history_size as AbsRow + line as AbsRow
    }

    /// Grid line for an absolute row given the current history size.
    /// Negative = in history, `>= rows` = below the screen (never for
    /// real rows, but the math is total).
    pub fn line(&self, history_size: usize, abs: AbsRow) -> i64 {
        abs - self.dropped - history_size as AbsRow
    }

    #[cfg(test)]
    pub fn dropped(&self) -> AbsRow {
        self.dropped
    }

    pub fn spans(&self) -> impl Iterator<Item = &Span> {
        self.spans.iter()
    }

    /// The live prompt's rows `(start, end)` inclusive, if the shell is
    /// waiting at a prompt (last span opened by `A`, no `C` yet).
    pub fn live_prompt(&self) -> Option<(AbsRow, AbsRow)> {
        let last = self.spans.back()?;
        if !last.at_prompt() {
            return None;
        }
        let end = last
            .prompt_end
            .map(|(row, _)| row)
            .unwrap_or(last.prompt_start)
            .max(last.prompt_start);
        Some((last.prompt_start, end))
    }

    /// OSC 133;A at `row`. If the current span never left the prompt
    /// (zle redraws the prompt on SIGWINCH, `clear-screen`, …) the
    /// redraw replaces it instead of stacking a ghost span.
    pub fn prompt_start(&mut self, row: AbsRow) {
        match self.spans.back_mut() {
            Some(last) if last.at_prompt() => *last = Span::new(row),
            _ => self.spans.push_back(Span::new(row)),
        }
    }

    /// OSC 133;B at `(row, col)`.
    pub fn prompt_end(&mut self, row: AbsRow, col: usize) {
        if let Some(last) = self.spans.back_mut()
            && last.at_prompt()
        {
            last.prompt_end = Some((row, col));
        }
    }

    /// OSC 133;C at `row`. Without an open prompt span (integration
    /// came alive mid-command) a header-less span starts here.
    pub fn output_start(&mut self, row: AbsRow) {
        let now = Instant::now();
        match self.spans.back_mut() {
            Some(last) if last.at_prompt() => {
                last.output_start = Some(row);
                last.started_at = Some(now);
            }
            _ => {
                let mut span = Span::new(row);
                span.output_start = Some(row);
                span.started_at = Some(now);
                self.spans.push_back(span);
            }
        }
    }

    /// OSC 133;D at `row`. A `D` with no open span (the first precmd of
    /// a session fires one before any prompt) is ignored.
    pub fn command_end(&mut self, row: AbsRow, exit_code: Option<i32>) {
        if let Some(last) = self.spans.back_mut()
            && !last.is_closed()
        {
            last.end = Some(row.max(last.prompt_start));
            last.exit_code = exit_code;
            last.duration = last.started_at.map(|t| t.elapsed());
        }
    }

    /// `n` lines were removed from the top of history (our own
    /// slack truncation). Absolute rows stay valid; spans that fell
    /// off entirely are pruned.
    pub fn history_dropped(&mut self, n: usize) {
        self.dropped += n as AbsRow;
        self.prune();
    }

    /// `CSI 3 J` — history of `history_before` lines wiped. Screen rows
    /// keep their identity; everything that lived in history is gone.
    pub fn history_cleared(&mut self, history_before: usize) {
        self.dropped += history_before as AbsRow;
        self.prune();
    }

    /// RIS — screen and history wiped. Nothing to decorate any more.
    pub fn reset(&mut self, history_before: usize) {
        self.dropped += history_before as AbsRow;
        self.spans.clear();
    }

    /// Row identity cannot survive a reflow (alacritty drops its own
    /// selection on column changes for the same reason). The next `A`
    /// re-syncs the live prompt.
    pub fn columns_changed(&mut self) {
        self.spans.clear();
    }

    /// History hit its ceiling inside one feed step, so some drops went
    /// uncounted. Everything recorded so far may be misaligned; start
    /// over from the next marker. `dropped` keeps its value — only
    /// relative consistency between rows recorded from here on matters.
    pub fn tracking_lost(&mut self) {
        self.spans.clear();
    }

    /// Drop closed spans whose rows have all left the retained window.
    /// Rows below `dropped` no longer exist anywhere.
    fn prune(&mut self) {
        while let Some(front) = self.spans.front() {
            if front.is_closed() && front.last_row() < self.dropped {
                self.spans.pop_front();
            } else {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_cycle_records_rows() {
        let mut t = SpanTracker::new();
        t.prompt_start(10);
        t.prompt_end(11, 2);
        assert_eq!(t.live_prompt(), Some((10, 11)));
        t.output_start(12);
        assert_eq!(t.live_prompt(), None, "command running — no live prompt");
        t.command_end(20, Some(1));
        let span = t.spans().next().copied().expect("one span");
        assert_eq!(span.prompt_start, 10);
        assert_eq!(span.prompt_end, Some((11, 2)));
        assert_eq!(span.output_start, Some(12));
        assert_eq!(span.end, Some(20));
        assert_eq!(span.exit_code, Some(1));
        assert_eq!(span.header_end(), 12);
    }

    #[test]
    fn prompt_redraw_replaces_live_span() {
        let mut t = SpanTracker::new();
        t.prompt_start(5);
        t.prompt_end(5, 4);
        // SIGWINCH redraw: A again without C in between.
        t.prompt_start(7);
        t.prompt_end(7, 4);
        assert_eq!(t.spans().count(), 1);
        assert_eq!(t.live_prompt(), Some((7, 7)));
    }

    #[test]
    fn empty_enter_closes_a_headerless_cycle() {
        let mut t = SpanTracker::new();
        t.prompt_start(0);
        t.prompt_end(0, 2);
        t.command_end(1, Some(0)); // precmd after an empty Enter
        t.prompt_start(1);
        assert_eq!(t.spans().count(), 2);
        let first = t.spans().next().unwrap();
        assert!(first.is_closed());
        assert_eq!(first.header_end(), 1, "header spans the prompt row only");
        assert_eq!(t.live_prompt(), Some((1, 1)));
    }

    #[test]
    fn stray_d_before_any_prompt_is_ignored() {
        let mut t = SpanTracker::new();
        t.command_end(0, Some(0));
        assert_eq!(t.spans().count(), 0);
    }

    #[test]
    fn abs_and_line_round_trip_across_history_growth() {
        let mut t = SpanTracker::new();
        // Marker on screen line 3 with 10 lines of history.
        let abs = t.abs(10, 3);
        assert_eq!(abs, 13);
        // 5 more lines pushed: history 15, the same content is now line -2.
        assert_eq!(t.line(15, abs), -2);
        // 4 lines dropped off the top: history back to 11, still line -2.
        t.history_dropped(4);
        assert_eq!(t.line(11, abs), -2);
    }

    #[test]
    fn history_cleared_kills_history_spans_keeps_screen_rows() {
        let mut t = SpanTracker::new();
        // Old span entirely in history (rows 0..3), history size 20.
        t.prompt_start(0);
        t.output_start(1);
        t.command_end(3, Some(0));
        // Live prompt on screen line 2 (abs = 22).
        let live = t.abs(20, 2);
        t.prompt_start(live);
        t.history_cleared(20);
        assert_eq!(t.spans().count(), 1, "history span pruned");
        // Screen row keeps its identity: history is now 0, line still 2.
        assert_eq!(t.line(0, live), 2);
    }

    #[test]
    fn reset_and_reflow_drop_everything() {
        let mut t = SpanTracker::new();
        t.prompt_start(0);
        t.output_start(1);
        t.command_end(5, Some(0));
        t.prompt_start(5);
        t.columns_changed();
        assert_eq!(t.spans().count(), 0);
        t.prompt_start(5);
        t.reset(3);
        assert_eq!(t.spans().count(), 0);
        assert_eq!(t.dropped(), 3);
    }

    #[test]
    fn prune_keeps_open_and_partially_retained_spans() {
        let mut t = SpanTracker::new();
        t.prompt_start(0);
        t.output_start(1);
        t.command_end(10, Some(0)); // rows 0..10
        t.prompt_start(10);
        t.output_start(11); // running
        t.history_dropped(5);
        assert_eq!(t.spans().count(), 2, "first span still has rows 5..10");
        t.history_dropped(5);
        assert_eq!(t.spans().count(), 1, "first span gone; running span kept");
    }
}
