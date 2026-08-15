//! Wraps alacritty_terminal::Term with a clean interface.
//!
//! Feeds PTY bytes through the vte parser into Term, then converts
//! the grid to a TerminalGrid for the renderer.

use std::sync::mpsc;

use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Boundary, Column, Direction, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::Config as TermConfig;
use alacritty_terminal::term::search::{Match, RegexSearch};
use alacritty_terminal::term::{Term, TermMode};
use alacritty_terminal::vte;
use flux_types::{CellData, CellFlags, Color, ResolvedTheme, TerminalGrid};

use crate::blocks::{BlockCapture, ShellPhase, StreamEvent};
use crate::spans::{AbsRow, Span, SpanTracker};

/// Extra history rows allocated above the configured scrollback. Lines
/// pushed into history are only observable while history is below its
/// ceiling (alacritty drops silently at the cap), so we keep the cap
/// this far above the configured size and truncate back down after
/// every feed step — the truncated count is exact. Must exceed the lines
/// one `FEED_STEP` can push (one per LF, so ≥ FEED_STEP), with headroom
/// for a row-shrinking resize (≤ screen rows). Memory: transient, up to
/// this many extra rows above the configured scrollback.
pub const HISTORY_SLACK: usize = 1024;

/// Bytes fed between history checks. Bounds the lines one step can push
/// (one per LF in real output) so it stays under `HISTORY_SLACK`.
const FEED_STEP: usize = 512;

/// How a mouse gesture groups cells — mapped onto alacritty's
/// selection machinery (which anchors to CONTENT in absolute
/// scrollback coordinates, so selections survive scrolling and can
/// span far more than one screen).
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum SelectMode {
    /// Cell-by-cell (click + drag).
    Char,
    /// Word-snapped (double-click).
    Word,
    /// Whole lines (triple-click).
    Line,
    /// Rectangular (Alt+drag).
    Block,
}

impl SelectMode {
    fn to_alacritty(self) -> SelectionType {
        match self {
            SelectMode::Char => SelectionType::Simple,
            SelectMode::Word => SelectionType::Semantic,
            SelectMode::Line => SelectionType::Lines,
            SelectMode::Block => SelectionType::Block,
        }
    }
}

/// Events that alacritty_terminal sends back (bell, title change, etc.)
#[derive(Debug)]
pub enum TermEvent {
    /// Write these bytes back to the PTY (terminal query responses).
    PtyWrite(String),
    /// Terminal bell.
    Bell,
    /// Window title changed.
    Title(String),
}

/// Event listener that captures alacritty_terminal events via channel.
struct EventProxy {
    tx: mpsc::Sender<TermEvent>,
}

impl EventListener for EventProxy {
    fn send_event(&self, event: Event) {
        match event {
            Event::PtyWrite(text) => {
                let _ = self.tx.send(TermEvent::PtyWrite(text));
            }
            Event::Title(title) => {
                let _ = self.tx.send(TermEvent::Title(title));
            }
            Event::Bell => {
                let _ = self.tx.send(TermEvent::Bell);
            }
            _ => {}
        }
    }
}

/// Terminal dimensions for alacritty_terminal.
struct TermDimensions {
    cols: usize,
    rows: usize,
}

impl Dimensions for TermDimensions {
    fn total_lines(&self) -> usize {
        self.rows
    }

    fn screen_lines(&self) -> usize {
        self.rows
    }

    fn columns(&self) -> usize {
        self.cols
    }

    fn last_column(&self) -> alacritty_terminal::index::Column {
        alacritty_terminal::index::Column(self.cols.saturating_sub(1))
    }

    fn topmost_line(&self) -> alacritty_terminal::index::Line {
        alacritty_terminal::index::Line(0)
    }

    fn bottommost_line(&self) -> alacritty_terminal::index::Line {
        alacritty_terminal::index::Line(self.rows as i32 - 1)
    }
}

/// Search state — the compiled query and the focused match (grid
/// coordinates; alacritty keeps them meaningful across scrolling only
/// while the content exists, so `focused` is re-resolved on demand).
struct SearchState {
    regex: RegexSearch,
    focused: Option<Match>,
}

/// Escape regex metacharacters so a query is matched literally.
fn regex_escape(query: &str) -> String {
    let mut out = String::with_capacity(query.len() + 8);
    for ch in query.chars() {
        if matches!(
            ch,
            '\\' | '.'
                | '+'
                | '*'
                | '?'
                | '('
                | ')'
                | '|'
                | '['
                | ']'
                | '{'
                | '}'
                | '^'
                | '$'
                | '#'
                | '&'
                | '-'
                | '~'
        ) {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

/// What the snapshot should do to the viewport: which live-prompt rows
/// (grid coordinates, inclusive) to hide, and how far to bump the
/// display offset to fill blank rows at the top with recent history.
#[derive(Debug, Clone, Copy)]
struct ViewPlan {
    hidden: Option<(i64, i64)>,
    bump: usize,
}

/// After a screen clear issued by a command that had printed at most
/// this many rows of output, the command's block stays in view (Warp
/// keeps the `clear` block). More output than this and the clear
/// starts a fresh view — a `cargo watch -c` loop should not drag its
/// previous run back in.
const CLEAR_KEEP_OUTPUT_ROWS: AbsRow = 2;

/// End (exclusive) of the side-parser run starting at `start`. A run
/// ends right *after* a byte that can terminate an OSC (`BEL`, `\\`,
/// `0x9c`) or right *before* a byte that introduces an escape sequence
/// (`ESC`, 8-bit DCS/CSI/OSC), so every control sequence sits at the
/// head of its run and every OSC completes exactly at a run end.
fn run_end(bytes: &[u8], start: usize) -> usize {
    let is_terminator = |b: u8| matches!(b, 0x07 | b'\\' | 0x9c);
    let is_introducer = |b: u8| matches!(b, 0x1b | 0x90 | 0x9b | 0x9d);
    // An introducer at the head belongs to this run.
    let scan = if is_introducer(bytes[start]) {
        start + 1
    } else {
        start
    };
    for (offset, &b) in bytes[scan..].iter().enumerate() {
        if is_terminator(b) {
            return scan + offset + 1;
        }
        if is_introducer(b) {
            return scan + offset;
        }
    }
    bytes.len()
}

/// Wraps alacritty_terminal with a clean API.
pub struct TerminalState {
    term: Term<EventProxy>,
    parser: vte::ansi::Processor,
    /// Side-channel OSC interceptor. See `blocks.rs` for the full
    /// rationale — in short, alacritty's ansi layer drops OSC 7 and
    /// OSC 133 before they reach `Term`, so we run a second parser
    /// over the same byte stream to catch them.
    block_capture: BlockCapture,
    /// Stock vte parser driving `block_capture`. Independent state
    /// machine from `parser` — both see the exact same `&[u8]` but
    /// neither affects the other.
    block_parser: vte::Parser,
    /// Absolute-row bookkeeping for prompt/command spans (v0.3a).
    tracker: SpanTracker,
    /// Configured scrollback; the grid is allocated `HISTORY_SLACK`
    /// above this and truncated back after every feed step.
    scrollback_cap: usize,
    /// Primary-grid history size after the last settle — the value to
    /// trust while the alt screen is active (its grid has no history).
    last_history: usize,
    /// Extra display offset the last snapshot applied (hidden prompt
    /// pulled off the bottom, blank top rows filled from history).
    /// Mouse mapping adds it.
    view_bump: usize,
    /// Oldest absolute row the snapshot may pull into view to fill
    /// blank rows. Advances on screen clears so a cleared screen stays
    /// cleared (except for the block that cleared it — see
    /// `CLEAR_KEEP_OUTPUT_ROWS`), history wipes, and resets.
    view_floor: AbsRow,
    /// Master switch for the semantic stream (prompt hiding + span
    /// decoration). Off = classic stream.
    blocks_enabled: bool,
    /// Active search (F14): compiled regex + the focused match.
    search: Option<SearchState>,
    /// Resolved color palette for named/indexed ANSI colors.
    theme: ResolvedTheme,
    event_rx: mpsc::Receiver<TermEvent>,
    cols: usize,
    rows: usize,
}

impl TerminalState {
    /// Create a new terminal state with the given dimensions,
    /// scrollback capacity in lines, and resolved color theme.
    pub fn new(cols: usize, rows: usize, scrollback_lines: usize, theme: ResolvedTheme) -> Self {
        let (tx, rx) = mpsc::channel();
        let event_proxy = EventProxy { tx };

        let config = TermConfig {
            scrolling_history: scrollback_lines + HISTORY_SLACK,
            ..TermConfig::default()
        };
        let dims = TermDimensions { cols, rows };
        let term = Term::new(config, &dims, event_proxy);
        let parser = vte::ansi::Processor::new();

        Self {
            term,
            parser,
            theme,
            block_capture: BlockCapture::new(),
            block_parser: vte::Parser::new(),
            tracker: SpanTracker::new(),
            scrollback_cap: scrollback_lines,
            last_history: 0,
            view_bump: 0,
            view_floor: 0,
            blocks_enabled: true,
            search: None,
            event_rx: rx,
            cols,
            rows,
        }
    }

    /// Enable or disable the semantic stream (live-prompt hiding and
    /// span decoration). Row tracking keeps running either way so
    /// flipping it on later has history to work with.
    pub fn set_blocks_enabled(&mut self, enabled: bool) {
        self.blocks_enabled = enabled;
    }

    pub fn blocks_enabled(&self) -> bool {
        self.blocks_enabled
    }

    /// Start or update a search. `query` is matched literally,
    /// case-insensitively. Returns false when the query is empty or
    /// cannot compile. The first match at or above the viewport's
    /// bottom is focused and scrolled into view.
    pub fn search_set(&mut self, query: &str) -> bool {
        if query.is_empty() {
            self.search = None;
            return false;
        }
        let pattern = format!("(?i){}", regex_escape(query));
        match RegexSearch::new(&pattern) {
            Ok(regex) => {
                self.search = Some(SearchState {
                    regex,
                    focused: None,
                });
                // Search backwards from the bottom of the live screen so
                // the newest match is focused first (like a pager's `?`).
                let origin = Point::new(
                    Line(self.rows as i32 - 1),
                    Column(self.cols.saturating_sub(1)),
                );
                self.search_step_from(origin, Direction::Left);
                true
            }
            Err(e) => {
                log::warn!("search regex failed to compile: {e}");
                self.search = None;
                false
            }
        }
    }

    pub fn search_clear(&mut self) {
        self.search = None;
    }

    // ---- block navigation (v0.3b seed) ----

    /// The command text of the block whose header row is at viewport
    /// `row` (the row as painted by the last snapshot). Reads the echo
    /// cells — from the prompt's end column through the header's last
    /// row — so it works for any prompt without knowing its shape.
    pub fn block_command_at_row(&self, row: usize) -> Option<String> {
        if !self.blocks_enabled || self.is_alt_screen() {
            return None;
        }
        let history = self.term.grid().history_size();
        let offset = self.display_offset() + self.view_bump;
        let abs = self.tracker.abs(history, row as i32 - offset as i32);
        let span = self
            .tracker
            .spans()
            .find(|s| !s.at_prompt() && s.prompt_start <= abs && abs < s.header_end())?;
        let (echo_row, echo_col) = span.prompt_end?;
        let mut text = String::new();
        for r in echo_row..span.header_end() {
            let line = self.tracker.line(history, r);
            let grid_row = &self.term.grid()[Line(line as i32)];
            let from = if r == echo_row { echo_col } else { 0 };
            for col in from..self.cols {
                let ch = grid_row[Column(col)].c;
                if ch != '\0' {
                    text.push(ch);
                }
            }
        }
        let text = text.trim().to_string();
        (!text.is_empty()).then_some(text)
    }

    /// Scroll so the previous (`-1`) / next (`+1`) block header sits at
    /// the top of the viewport. Returns false when there is none.
    pub fn scroll_to_block(&mut self, step: i32) -> bool {
        if !self.blocks_enabled || self.is_alt_screen() {
            return false;
        }
        let history = self.term.grid().history_size();
        let offset = self.display_offset() as i64;
        // Absolute row currently at the top of the viewport.
        let top = self.tracker.abs(history, -(offset as i32));
        let target = if step < 0 {
            // Nearest header above the viewport top. Headers already in
            // view can't be brought to the top (the viewport can't
            // scroll below the tail), so they're not "previous".
            self.tracker
                .spans()
                .filter(|s| !s.at_prompt() && s.prompt_start < top)
                .map(|s| s.prompt_start)
                .max()
        } else {
            self.tracker
                .spans()
                .filter(|s| !s.at_prompt() && s.prompt_start > top)
                .map(|s| s.prompt_start)
                .min()
        };
        let Some(target) = target else { return false };
        let line = self.tracker.line(history, target);
        // offset that puts `line` at viewport row 0: row = line + offset.
        let want = (-line).clamp(0, history as i64);
        self.term
            .scroll_display(Scroll::Delta((want - offset) as i32));
        true
    }

    pub fn search_active(&self) -> bool {
        self.search.is_some()
    }

    /// Focus the next match below the current one (wraps).
    pub fn search_next(&mut self) {
        let Some(focused) = self.search.as_ref().and_then(|s| s.focused.clone()) else {
            let origin = Point::new(Line(-(self.term.grid().history_size() as i32)), Column(0));
            self.search_step_from(origin, Direction::Right);
            return;
        };
        let origin = focused.end().add(&self.term, Boundary::Grid, 1);
        self.search_step_from(origin, Direction::Right);
    }

    /// Focus the previous match above the current one (wraps).
    pub fn search_prev(&mut self) {
        let Some(focused) = self.search.as_ref().and_then(|s| s.focused.clone()) else {
            let origin = Point::new(
                Line(self.rows as i32 - 1),
                Column(self.cols.saturating_sub(1)),
            );
            self.search_step_from(origin, Direction::Left);
            return;
        };
        let origin = focused.start().sub(&self.term, Boundary::Grid, 1);
        self.search_step_from(origin, Direction::Left);
    }

    fn search_step_from(&mut self, origin: Point, direction: Direction) {
        let Some(search) = self.search.as_mut() else {
            return;
        };
        let side = match direction {
            Direction::Right => Side::Left,
            Direction::Left => Side::Right,
        };
        let found = self
            .term
            .search_next(&mut search.regex, origin, direction, side, None);
        search.focused = found.clone();
        if let Some(m) = found {
            self.scroll_line_into_view(m.start().line.0);
        }
    }

    /// Scroll so grid `line` sits inside the viewport (centered when it
    /// was outside). No-op if already visible.
    fn scroll_line_into_view(&mut self, line: i32) {
        let offset = self.display_offset() as i32;
        let rows = self.rows as i32;
        let top = -offset;
        if line >= top && line < top + rows {
            return;
        }
        let want_offset = (rows / 2 - line).max(0);
        self.term
            .scroll_display(Scroll::Delta(want_offset - offset));
    }

    /// `(position_of_focused, total)` match counts over the whole
    /// buffer, for the search bar's `n/N`. Walks the buffer once per
    /// call — fine for a UI refresh, and the walk stops the moment the
    /// wrap-around comes back to an earlier match.
    pub fn search_status(&self) -> Option<(Option<usize>, usize)> {
        let search = self.search.as_ref()?;
        let mut regex = search.regex.clone();
        let mut count = 0usize;
        let mut position: Option<usize> = None;
        let mut origin = Point::new(Line(-(self.term.grid().history_size() as i32)), Column(0));
        let bottom = Point::new(
            Line(self.rows as i32 - 1),
            Column(self.cols.saturating_sub(1)),
        );
        while let Some(m) = self.term.regex_search_right(&mut regex, origin, bottom) {
            if *m.start() < origin {
                break; // wrapped
            }
            count += 1;
            if search.focused.as_ref() == Some(&m) {
                position = Some(count);
            }
            if *m.end() >= bottom {
                break;
            }
            origin = m.end().add(&self.term, Boundary::Grid, 1);
        }
        Some((position, count))
    }

    /// Feed raw PTY output bytes into the terminal parser.
    ///
    /// Two parsers run over the same bytes:
    /// - `self.parser` drives alacritty's grid, cursor, and scrollback.
    /// - `self.block_parser` drives `BlockCapture`, a stock `vte::Parser`
    ///   that only exists to intercept OSC 7 / OSC 133 (and the two
    ///   history-wiping controls). See `blocks.rs` for why alacritty's
    ///   own layer can't see them.
    ///
    /// The side parser runs *first*, in runs split at bytes that can end
    /// an OSC (`BEL`, `\\`, `0x9c`) or start an escape sequence. When a
    /// run fires an event, the main parser is caught up to that exact
    /// byte before the event is acted on, so the cursor row and history
    /// size read at that moment are the ones the marker landed on. Runs
    /// that fire nothing cost one cheap side-parser call; the main parser
    /// still sees the bytes in bulk.
    ///
    /// Bytes are also stepped in `FEED_STEP` slices with a history settle
    /// between them — see `HISTORY_SLACK`.
    pub fn process_bytes(&mut self, bytes: &[u8]) {
        for step in bytes.chunks(FEED_STEP) {
            self.feed_step(step);
            self.settle_history();
        }
    }

    fn feed_step(&mut self, bytes: &[u8]) {
        let mut main_start = 0;
        let mut i = 0;
        while i < bytes.len() {
            let end = run_end(bytes, i);
            self.block_parser
                .advance(&mut self.block_capture, &bytes[i..end]);
            if self.block_capture.has_events() {
                // "Before" state = the main parser caught up to the run
                // start; the run itself holds at most one control
                // sequence and it sits at the run's head, so nothing in
                // the run precedes it.
                self.parser.advance(&mut self.term, &bytes[main_start..i]);
                let history_before = self.tracked_history();
                self.parser.advance(&mut self.term, &bytes[i..end]);
                main_start = end;
                let events = self.block_capture.take_events();
                self.apply_stream_events(&events, history_before);
            }
            i = end;
        }
        self.parser.advance(&mut self.term, &bytes[main_start..]);
    }

    /// Primary-grid history size — live when the primary grid is
    /// active, the last settled value while an alt-screen program owns
    /// the display (the alt grid reports 0).
    fn tracked_history(&self) -> usize {
        if self.is_alt_screen() {
            self.last_history
        } else {
            self.term.grid().history_size()
        }
    }

    /// Absolute row of the top screen line right now.
    fn screen_top(&self) -> AbsRow {
        self.tracker.abs(self.term.grid().history_size(), 0)
    }

    /// Record marker rows / apply history wipes for events the side
    /// parser just fired, with the main parser caught up to them.
    fn apply_stream_events(&mut self, events: &[StreamEvent], history_before: usize) {
        for &event in events {
            match event {
                StreamEvent::ScreenCleared => {
                    if self.is_alt_screen() {
                        continue;
                    }
                    let screen_top = self.screen_top();
                    // Keep the clearing command's block in view when it
                    // is a `clear`-like command (header + a couple of
                    // rows at most); otherwise the view restarts here.
                    let keep_from = self.tracker.spans().last().and_then(|span| {
                        let executing = span.output_start.is_some() && !span.is_closed();
                        let output_rows = screen_top - span.header_end();
                        (executing && output_rows <= CLEAR_KEEP_OUTPUT_ROWS)
                            .then_some(span.prompt_start)
                    });
                    self.view_floor = keep_from.unwrap_or(screen_top);
                }
                StreamEvent::HistoryCleared => {
                    if !self.is_alt_screen() {
                        self.tracker.history_cleared(history_before);
                        self.view_floor = self.screen_top();
                    }
                }
                StreamEvent::Reset => {
                    // RIS also leaves the alt screen, so by now the
                    // primary grid is active (and empty).
                    self.tracker.reset(history_before);
                    self.view_floor = self.screen_top();
                }
                marker => {
                    // Markers only mean something on the primary grid.
                    if self.is_alt_screen() {
                        continue;
                    }
                    let cursor = self.term.grid().cursor.point;
                    let row = self
                        .tracker
                        .abs(self.term.grid().history_size(), cursor.line.0);
                    match marker {
                        StreamEvent::PromptStart => self.tracker.prompt_start(row),
                        StreamEvent::PromptEnd => self.tracker.prompt_end(row, cursor.column.0),
                        StreamEvent::OutputStart => self.tracker.output_start(row),
                        StreamEvent::CommandEnd(code) => self.tracker.command_end(row, code),
                        StreamEvent::ScreenCleared
                        | StreamEvent::HistoryCleared
                        | StreamEvent::Reset => unreachable!(),
                    }
                }
            }
        }
    }

    /// Keep history observable: truncate anything above the configured
    /// cap (counting exactly what went), and notice if the ceiling was
    /// hit — meaning some drops went uncounted.
    fn settle_history(&mut self) {
        if self.is_alt_screen() {
            return;
        }
        let history = self.term.grid().history_size();
        if history >= self.scrollback_cap + HISTORY_SLACK {
            log::warn!(
                "history ceiling hit in one feed step ({history} lines); span tracking reset"
            );
            self.tracker.tracking_lost();
        }
        if history > self.scrollback_cap {
            let excess = history - self.scrollback_cap;
            let grid = self.term.grid_mut();
            grid.update_history(self.scrollback_cap);
            grid.update_history(self.scrollback_cap + HISTORY_SLACK);
            self.tracker.history_dropped(excess);
        }
        self.last_history = self.term.grid().history_size();
    }

    pub fn cols(&self) -> usize {
        self.cols
    }

    pub fn rows(&self) -> usize {
        self.rows
    }

    /// Scroll the display by `lines` (positive = up into history,
    /// negative = down towards the live tail). Alacritty clamps at
    /// both ends, so overshooting is a no-op.
    pub fn scroll_lines(&mut self, lines: i32) {
        self.term.scroll_display(Scroll::Delta(lines));
    }

    pub fn scroll_page_up(&mut self) {
        self.term.scroll_display(Scroll::PageUp);
    }

    pub fn scroll_page_down(&mut self) {
        self.term.scroll_display(Scroll::PageDown);
    }

    pub fn scroll_to_bottom(&mut self) {
        self.term.scroll_display(Scroll::Bottom);
    }

    /// Current history offset in lines. 0 = tailing live output;
    /// positive = the user has scrolled that many lines into history.
    /// When new output arrives while scrolled up, alacritty grows the
    /// offset internally so the viewport doesn't jump — no gate needed
    /// on our side.
    pub fn display_offset(&self) -> usize {
        self.term.grid().display_offset()
    }

    /// Convert a viewport cell to alacritty grid coordinates (absolute
    /// within the visible+history window): grid line = viewport row −
    /// (display offset + the hidden-prompt bump the last snapshot
    /// applied, so clicks land on what was actually painted).
    fn viewport_to_point(&self, col: usize, row: usize) -> Point {
        let offset = self.display_offset() + self.view_bump;
        let line = Line(row as i32 - offset as i32);
        Point::new(line, Column(col.min(self.cols.saturating_sub(1))))
    }

    /// Begin a selection at a viewport cell. `right_side` picks the
    /// half of the cell the pointer landed in (char-precise edges).
    pub fn start_selection(&mut self, mode: SelectMode, col: usize, row: usize, right_side: bool) {
        let point = self.viewport_to_point(col, row);
        let side = if right_side { Side::Right } else { Side::Left };
        self.term.selection = Some(Selection::new(mode.to_alacritty(), point, side));
    }

    /// Extend the active selection to a viewport cell (drag /
    /// Shift+click).
    pub fn update_selection(&mut self, col: usize, row: usize, right_side: bool) {
        let point = self.viewport_to_point(col, row);
        let side = if right_side { Side::Right } else { Side::Left };
        if let Some(selection) = &mut self.term.selection {
            selection.update(point, side);
        }
    }

    pub fn clear_terminal_selection(&mut self) {
        self.term.selection = None;
    }

    pub fn has_selection(&self) -> bool {
        self.term.selection.is_some()
    }

    /// The selected text, across scrollback if the selection spans it.
    /// None when there's no selection or it's empty (a click that
    /// never dragged).
    pub fn selection_text(&self) -> Option<String> {
        self.term.selection_to_string().filter(|s| !s.is_empty())
    }

    /// Drain any events from alacritty_terminal (PtyWrite, Bell, Title).
    pub fn drain_events(&self) -> Vec<TermEvent> {
        let mut events = Vec::new();
        while let Ok(event) = self.event_rx.try_recv() {
            events.push(event);
        }
        events
    }

    /// Snapshot the current terminal grid for rendering.
    ///
    /// With the semantic stream on and the shell waiting at a prompt,
    /// the live prompt rows are pulled off the bottom of the viewport
    /// (see `HiddenPrompt`) and `grid.cursor` reports the row above them
    /// as the bottom-anchor row. Completed spans get their header rows
    /// tinted, the command echo bold, and failed commands a red tint plus
    /// `✘ code` at the right edge. `grid.display_offset` stays the
    /// user's own scroll offset — the hidden-prompt bump is invisible to
    /// scroll logic and only folded into mouse mapping via
    /// `viewport_to_point`.
    pub fn grid_snapshot(&mut self) -> TerminalGrid {
        let user_offset = self.display_offset();
        let plan = self.plan_view();
        self.view_bump = 0;
        if plan.bump > 0 {
            self.term.scroll_display(Scroll::Delta(plan.bump as i32));
            self.view_bump = self.display_offset() - user_offset;
        }

        let mut grid = self.snapshot_viewport();
        grid.display_offset = user_offset;

        if self.blocks_enabled && !self.is_alt_screen() {
            let offset = user_offset + self.view_bump;
            self.decorate_spans(&mut grid, offset);
            if let Some(hidden) = plan.hidden {
                self.blank_hidden_prompt(&mut grid, hidden, offset, user_offset == 0);
            }
        }
        self.flag_search_matches(&mut grid, user_offset + self.view_bump);

        if self.view_bump > 0 {
            self.term
                .scroll_display(Scroll::Delta(-(self.view_bump as i32)));
        }
        grid
    }

    /// Plain viewport → `TerminalGrid` conversion at alacritty's current
    /// display offset.
    fn snapshot_viewport(&self) -> TerminalGrid {
        let content = self.term.renderable_content();
        let mut grid = TerminalGrid::new(self.cols, self.rows);
        // Alacritty's display_iter yields points in GRID coordinates,
        // where scrolled-into-history rows have NEGATIVE line numbers
        // (line 0 is the top of the live screen, -1 the line above it).
        // Viewport row = grid line + display_offset. Getting this wrong
        // renders a scrolled view as blank — regression-tested below.
        let display_offset = content.display_offset as i32;
        grid.display_offset = content.display_offset;
        grid.history_size = self.term.grid().history_size();

        // Set cursor position (scrolled up, the cursor converts to a row
        // at/below the viewport bottom and is culled by the bounds check).
        let cursor_point = content.cursor.point;
        let cursor_col = cursor_point.column.0;
        let cursor_row = cursor_point.line.0 + display_offset;
        if cursor_col < self.cols && (0..self.rows as i32).contains(&cursor_row) {
            grid.cursor = Some((cursor_col, cursor_row as usize));
        }
        grid.cursor_hidden = matches!(
            content.cursor.shape,
            alacritty_terminal::vte::ansi::CursorShape::Hidden
        );

        // Selection range in grid coordinates — alacritty resolves the
        // content-anchored selection against the current viewport.
        let selection_range = content.selection;

        for cell in content.display_iter {
            let col = cell.point.column.0;
            let row_i = cell.point.line.0 + display_offset;
            if row_i < 0 {
                continue;
            }
            let row = row_i as usize;

            if col >= self.cols || row >= self.rows {
                continue;
            }

            let selected = selection_range
                .map(|range| range.contains(cell.point))
                .unwrap_or(false);

            let fg = self.convert_color(cell.fg);
            let bg = self.convert_color(cell.bg);

            let mut flags = CellFlags::empty();
            if selected {
                flags |= CellFlags::SELECTION;
            }
            use alacritty_terminal::term::cell::Flags;
            if cell.flags.contains(Flags::BOLD) {
                flags |= CellFlags::BOLD;
            }
            if cell.flags.contains(Flags::ITALIC) {
                flags |= CellFlags::ITALIC;
            }
            if cell.flags.contains(Flags::UNDERLINE) {
                flags |= CellFlags::UNDERLINE;
            }
            if cell.flags.contains(Flags::HIDDEN) {
                flags |= CellFlags::HIDDEN;
            }
            if cell.flags.contains(Flags::DIM_BOLD) {
                flags |= CellFlags::DIM;
            }
            if cell.flags.contains(Flags::WIDE_CHAR) {
                flags |= CellFlags::WIDE_CHAR;
            }

            grid.set(
                row,
                col,
                CellData {
                    character: cell.c,
                    fg,
                    bg,
                    flags,
                },
            );
        }

        grid
    }

    /// Decide what the snapshot shows. With the semantic stream on:
    /// the live prompt's rows are hidden, and blank rows that would sit
    /// above the content (the renderer bottom-anchors on the last
    /// content row) are filled with recent history — never older than
    /// `view_floor`, so a cleared screen stays cleared and a full
    /// screen stays continuous.
    fn plan_view(&self) -> ViewPlan {
        let none = ViewPlan {
            hidden: None,
            bump: 0,
        };
        if !self.blocks_enabled || self.is_alt_screen() {
            return none;
        }
        let history = self.term.grid().history_size();
        let rows = self.rows as i64;
        let hidden = self.tracker.live_prompt().map(|(start, end)| {
            (
                self.tracker.line(history, start),
                self.tracker.line(history, end),
            )
        });
        // Last row holding content once the prompt is hidden — the row
        // the renderer will anchor at the bottom.
        let last_content_line = match hidden {
            Some((start_line, _)) if (0..rows).contains(&start_line) => start_line - 1,
            _ => self.term.grid().cursor.point.line.0 as i64,
        };
        let blank_top = (rows - 1 - last_content_line).clamp(0, rows);
        let screen_top = self.tracker.abs(history, 0);
        let max_pull = (screen_top - self.view_floor).clamp(0, history as i64);
        ViewPlan {
            hidden,
            bump: blank_top.min(max_pull) as usize,
        }
    }

    /// Blank the live prompt rows still inside the viewport and move the
    /// bottom-anchor row to the last row above them.
    fn blank_hidden_prompt(
        &self,
        grid: &mut TerminalGrid,
        hidden: (i64, i64),
        offset: usize,
        at_tail: bool,
    ) {
        let blank = CellData {
            character: ' ',
            fg: self.theme.foreground,
            bg: self.theme.background,
            flags: CellFlags::empty(),
        };
        let (start_line, end_line) = hidden;
        let first_row = start_line + offset as i64;
        let last_row = end_line + offset as i64;
        for row in first_row.max(0)..=last_row.min(self.rows as i64 - 1) {
            for col in 0..self.cols {
                grid.set(row as usize, col, blank);
            }
        }
        // Anchor: bottom-hugging content against the input bar is a
        // TAIL-ONLY presentation. The moment the user scrolls, the view
        // must be a plain window over the buffer — old content enters
        // at the top edge and slides down per notch, like every
        // terminal. Keeping the anchor while scrolled pinned content to
        // the bottom edge instead, which read as "a cover being lifted"
        // (dogfood, 2026-08-15). At the tail: the row above the prompt
        // when it's in view; when the prompt sits below the viewport
        // (bump pulled it off) the whole viewport is content and the
        // renderer's default anchor (bottom row) is right; at the very
        // top there is no content to anchor.
        grid.cursor = if !at_tail || first_row >= self.rows as i64 {
            None
        } else if first_row > 0 {
            Some((0, first_row as usize - 1))
        } else {
            None
        };
    }

    /// Set SEARCH_MATCH on every visible match cell and SEARCH_FOCUS on
    /// the focused one. Walks only the viewport's rows.
    fn flag_search_matches(&self, grid: &mut TerminalGrid, offset: usize) {
        let Some(search) = self.search.as_ref() else {
            return;
        };
        let mut regex = search.regex.clone();
        let top = Point::new(Line(-(offset as i32)), Column(0));
        let bottom = Point::new(
            Line(self.rows as i32 - 1 - offset as i32),
            Column(self.cols.saturating_sub(1)),
        );
        let mut origin = top;
        while let Some(m) = self.term.regex_search_right(&mut regex, origin, bottom) {
            if *m.start() < origin || *m.start() > bottom {
                break;
            }
            let focused = search.focused.as_ref() == Some(&m);
            let mut point = *m.start();
            loop {
                let row = point.line.0 + offset as i32;
                if (0..self.rows as i32).contains(&row) && point.column.0 < self.cols {
                    let mut cell = *grid.get(row as usize, point.column.0);
                    cell.flags |= CellFlags::SEARCH_MATCH;
                    if focused {
                        cell.flags |= CellFlags::SEARCH_FOCUS;
                    }
                    grid.set(row as usize, point.column.0, cell);
                }
                if point >= *m.end() {
                    break;
                }
                point = point.add(&self.term, Boundary::Grid, 1);
            }
            if *m.end() >= bottom {
                break;
            }
            origin = m.end().add(&self.term, Boundary::Grid, 1);
        }
    }

    /// Tint completed / running spans' header rows, embolden the echo,
    /// and mark failures.
    fn decorate_spans(&self, grid: &mut TerminalGrid, offset: usize) {
        let history = self.term.grid().history_size();
        let rows = self.rows as i64;
        // Visible absolute rows: [lo, hi).
        let lo = self.tracker.abs(history, -(offset as i32));
        let hi = lo + rows;
        for span in self.tracker.spans() {
            if span.at_prompt() {
                continue; // the live prompt is hidden, not decorated
            }
            let header_end = span.header_end();
            if header_end <= lo || span.prompt_start >= hi {
                continue;
            }
            self.decorate_header(grid, span, lo, header_end);
        }
    }

    fn decorate_header(
        &self,
        grid: &mut TerminalGrid,
        span: &Span,
        lo: AbsRow,
        header_end: AbsRow,
    ) {
        let failed = span.exit_code.is_some_and(|code| code != 0);
        let tint = if failed {
            self.theme.block_failed
        } else {
            self.theme.block_header
        };
        let default_bg = self.theme.background;
        let rows = self.rows as i64;

        for abs in span.prompt_start.max(lo)..header_end.min(lo + rows) {
            let row = (abs - lo) as usize;
            for col in 0..self.cols {
                let cell = grid.get(row, col);
                if cell.bg == default_bg {
                    let mut tinted = *cell;
                    tinted.bg = tint;
                    grid.set(row, col, tinted);
                }
            }
        }

        // Command echo: from the prompt's end through the last header row.
        if let Some((echo_row, echo_col)) = span.prompt_end {
            for abs in echo_row.max(lo)..header_end.min(lo + rows) {
                let row = (abs - lo) as usize;
                let from = if abs == echo_row { echo_col } else { 0 };
                for col in from..self.cols {
                    let mut cell = *grid.get(row, col);
                    cell.flags |= CellFlags::BOLD;
                    grid.set(row, col, cell);
                }
            }
        }

        // `✘ code` at the right edge of the first header row, only if
        // that space is blank (a right prompt lives there otherwise).
        if failed && span.prompt_start >= lo && span.prompt_start < lo + rows {
            let row = (span.prompt_start - lo) as usize;
            let label = format!("✘ {}", span.exit_code.unwrap_or(0));
            let width = label.chars().count() + 1;
            if width <= self.cols {
                let start_col = self.cols - width;
                let blank = (start_col..self.cols).all(|col| grid.get(row, col).character == ' ');
                if blank {
                    for (i, ch) in label.chars().enumerate() {
                        let mut cell = *grid.get(row, start_col + i);
                        cell.character = ch;
                        cell.fg = self.theme.ansi(1);
                        cell.flags |= CellFlags::BOLD;
                        grid.set(row, start_col + i, cell);
                    }
                }
            }
        }
    }

    /// The shell's current working directory, if known via OSC 7.
    /// Returns `None` until the shell emits its first OSC 7 sequence.
    pub fn cwd(&self) -> Option<&std::path::Path> {
        self.block_capture.cwd()
    }

    /// Exit code of the last finished command, if known via OSC 133;D.
    pub fn last_exit_code(&self) -> Option<i32> {
        self.block_capture.last_exit_code()
    }

    /// True when the program on the other end of the PTY is using the
    /// alternate screen buffer — vim, less, man, htop, tmux all set this
    /// bit. It's the single most reliable signal that the user is in a
    /// full-screen program that owns the keyboard.
    pub fn is_alt_screen(&self) -> bool {
        self.term.mode().contains(TermMode::ALT_SCREEN)
    }

    /// True when the child program has enabled bracketed paste mode — the
    /// terminal should wrap pasted text in `\x1b[200~` / `\x1b[201~` so the
    /// program can distinguish it from typed input. Most shells and vim
    /// enable this by default.
    pub fn is_bracketed_paste(&self) -> bool {
        self.term.mode().contains(TermMode::BRACKETED_PASTE)
    }

    /// True when the child program has requested xterm mouse reporting
    /// (vim with `mouse=a`, htop, …). Local mouse selection defers to
    /// the program in that case.
    pub fn wants_mouse_reporting(&self) -> bool {
        self.term.mode().intersects(TermMode::MOUSE_MODE)
    }

    /// True when wheel events over the alt screen should be translated
    /// to arrow keys (DECSET 1007 — on by default in alacritty's mode,
    /// cleared by programs that want raw wheel control).
    pub fn alternate_scroll(&self) -> bool {
        self.term.mode().contains(TermMode::ALTERNATE_SCROLL)
    }

    /// True when the program requested the SGR mouse encoding
    /// (DECSET 1006) — modern programs all do; the legacy `\x1b[M`
    /// byte encoding is the fallback.
    pub fn sgr_mouse(&self) -> bool {
        self.term.mode().contains(TermMode::SGR_MOUSE)
    }

    /// True when the program wants drag events while a button is held
    /// (DECSET 1002 button-event or 1003 any-event tracking).
    pub fn reports_mouse_drag(&self) -> bool {
        self.term
            .mode()
            .intersects(TermMode::MOUSE_DRAG | TermMode::MOUSE_MOTION)
    }

    /// True when the program requested hover motion with no button held
    /// (DECSET 1003 any-event tracking only — Claude Code enables this).
    pub fn reports_mouse_motion(&self) -> bool {
        self.term.mode().contains(TermMode::MOUSE_MOTION)
    }

    /// True when the application cursor-keys mode is active (DECCKM) —
    /// arrow keys must then be encoded as `\x1bOA`-style sequences.
    pub fn app_cursor_keys(&self) -> bool {
        self.term.mode().contains(TermMode::APP_CURSOR)
    }

    /// True while a command is running (between OSC 133;C and 133;D).
    /// Keyboard routing sends keys straight to the PTY during this
    /// window so interactive programs that never touch the alt screen
    /// (sudo prompts, REPLs) receive keystrokes directly.
    pub fn is_executing(&self) -> bool {
        self.block_capture.integration_active()
            && self.block_capture.shell_phase() == ShellPhase::Executing
    }

    #[cfg(test)]
    pub(crate) fn tracker(&self) -> &SpanTracker {
        &self.tracker
    }

    #[cfg(test)]
    pub(crate) fn tracker_live_prompt(&self) -> Option<(AbsRow, AbsRow)> {
        self.tracker.live_prompt()
    }

    #[cfg(test)]
    pub(crate) fn debug_spans(&self) -> Vec<Span> {
        self.tracker.spans().copied().collect()
    }

    /// Text of one grid line (grid coordinates: 0 = top of screen,
    /// negative = history), trailing blanks trimmed.
    #[cfg(test)]
    pub(crate) fn row_text(&self, line: i32) -> String {
        let row = &self.term.grid()[Line(line)];
        (0..self.cols)
            .map(|c| row[Column(c)].c)
            .collect::<String>()
            .trim_end()
            .to_string()
    }

    /// `row_text` addressed by absolute row.
    #[cfg(test)]
    pub(crate) fn debug_row_text_abs(&self, abs: AbsRow) -> String {
        let history = self.term.grid().history_size();
        self.row_text(self.tracker.line(history, abs) as i32)
    }

    /// Text of a viewport row of a snapshot, trailing blanks trimmed.
    #[cfg(test)]
    fn grid_row_text(grid: &TerminalGrid, row: usize) -> String {
        (0..grid.cols)
            .map(|c| grid.get(row, c).character)
            .collect::<String>()
            .trim_end()
            .to_string()
    }

    /// Resize the terminal grid. A column change reflows history, which
    /// no row identity survives (alacritty drops its selection for the
    /// same reason) — spans are cleared and the live prompt re-marks on
    /// zle's redraw. A row-only change moves lines between screen and
    /// history without creating or destroying content rows, so absolute
    /// rows stay valid (regression-tested below).
    pub fn resize(&mut self, cols: usize, rows: usize) {
        let cols_changed = cols != self.cols;
        self.cols = cols;
        self.rows = rows;
        let dims = TermDimensions { cols, rows };
        self.term.resize(dims);
        if cols_changed {
            self.tracker.columns_changed();
            // Row identity above the screen is gone; don't pull any of
            // it into view until fresh output has scrolled past.
            self.view_floor = self.screen_top();
        }
        // Shrinking rows pushes lines into history — count them.
        self.settle_history();
    }

    /// Convert an alacritty color to our Color type.
    fn convert_color(&self, color: alacritty_terminal::vte::ansi::Color) -> Color {
        match color {
            alacritty_terminal::vte::ansi::Color::Named(named) => self.named_color(named),
            alacritty_terminal::vte::ansi::Color::Spec(rgb) => Color::from_rgb(rgb.r, rgb.g, rgb.b),
            alacritty_terminal::vte::ansi::Color::Indexed(idx) => self.indexed_color(idx),
        }
    }

    /// Map named ANSI colors through the resolved theme.
    fn named_color(&self, color: alacritty_terminal::vte::ansi::NamedColor) -> Color {
        use alacritty_terminal::vte::ansi::NamedColor::*;
        let t = &self.theme;
        match color {
            Black => t.ansi(0),
            Red => t.ansi(1),
            Green => t.ansi(2),
            Yellow => t.ansi(3),
            Blue => t.ansi(4),
            Magenta => t.ansi(5),
            Cyan => t.ansi(6),
            White => t.ansi(7),
            BrightBlack => t.ansi(8),
            BrightRed => t.ansi(9),
            BrightGreen => t.ansi(10),
            BrightYellow => t.ansi(11),
            BrightBlue => t.ansi(12),
            BrightMagenta => t.ansi(13),
            BrightCyan => t.ansi(14),
            BrightWhite => t.ansi(15),
            Foreground => t.foreground,
            Background => t.background,
            Cursor => t.cursor,
            _ => t.foreground,
        }
    }

    /// Map 256-color index to RGB.
    fn indexed_color(&self, idx: u8) -> Color {
        match idx {
            0..=15 => {
                // Standard colors — same as named
                use alacritty_terminal::vte::ansi::NamedColor;
                let named = match idx {
                    0 => NamedColor::Black,
                    1 => NamedColor::Red,
                    2 => NamedColor::Green,
                    3 => NamedColor::Yellow,
                    4 => NamedColor::Blue,
                    5 => NamedColor::Magenta,
                    6 => NamedColor::Cyan,
                    7 => NamedColor::White,
                    8 => NamedColor::BrightBlack,
                    9 => NamedColor::BrightRed,
                    10 => NamedColor::BrightGreen,
                    11 => NamedColor::BrightYellow,
                    12 => NamedColor::BrightBlue,
                    13 => NamedColor::BrightMagenta,
                    14 => NamedColor::BrightCyan,
                    15 => NamedColor::BrightWhite,
                    _ => unreachable!(),
                };
                self.named_color(named)
            }
            16..=231 => {
                // 216-color cube
                let idx = idx - 16;
                let r = (idx / 36) * 51;
                let g = ((idx % 36) / 6) * 51;
                let b = (idx % 6) * 51;
                Color::from_rgb(r, g, b)
            }
            232..=255 => {
                // Grayscale ramp
                let gray = 8 + (idx - 232) * 10;
                Color::from_rgb(gray, gray, gray)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// R3 smoke test — prove that `process_bytes` feeds BOTH parsers
    /// (main alacritty + side BlockCapture) without panicking, and
    /// that constructing a TerminalState works with the side parser
    /// wired in. No assertion on BlockCapture state — the side
    /// parser is a no-op in R3; F4 adds a test that verifies OSC 7
    /// actually updates `cwd`.
    #[test]
    fn block_capture_runs_alongside_main_parser() {
        let mut state = TerminalState::new(80, 24, 1000, ResolvedTheme::default());
        state.process_bytes(b"hello world\n");
        state.process_bytes(b"\x1b[31mred\x1b[0m\n");
        // Feed an OSC 7 sequence — the side parser should accept it
        // without panicking even though nothing reads the state yet.
        state.process_bytes(b"\x1b]7;file://localhost/tmp\x07");
    }

    #[test]
    fn scrollback_holds_history_and_offset_tracks_scrolling() {
        let mut state = TerminalState::new(80, 24, 1000, ResolvedTheme::default());
        // Push well past one screen of output.
        for i in 0..100 {
            state.process_bytes(format!("line {}\r\n", i).as_bytes());
        }
        assert_eq!(state.display_offset(), 0, "tailing by default");

        state.scroll_lines(10);
        assert_eq!(state.display_offset(), 10);

        // Overshoot clamps rather than panics.
        state.scroll_lines(100_000);
        let clamped = state.display_offset();
        assert!(clamped >= 10, "offset should clamp at history top");

        // New output while scrolled up must NOT move the viewport:
        // alacritty grows the offset to keep the same lines on screen.
        state.process_bytes(b"new tail line\r\n");
        assert_eq!(state.display_offset(), clamped + 1);

        state.scroll_to_bottom();
        assert_eq!(state.display_offset(), 0);

        // The snapshot carries the offset for downstream consumers.
        let grid = state.grid_snapshot();
        assert_eq!(grid.display_offset, 0);
    }

    /// Regression test for the "scrolled view renders black" bug:
    /// display_iter points are grid coords where history rows are
    /// NEGATIVE lines; the snapshot must convert to viewport rows.
    #[test]
    fn scrolled_snapshot_shows_history_content() {
        let mut state = TerminalState::new(80, 24, 1000, ResolvedTheme::default());
        for i in 0..30 {
            state.process_bytes(format!("line {}\r\n", i).as_bytes());
        }

        // Tailing: 31 logical rows, viewport shows rows 7.. => top = "line 7".
        let grid = state.grid_snapshot();
        let top: String = (0..7).map(|c| grid.get(0, c).character).collect();
        assert_eq!(top.trim_end(), "line 7");

        // Scroll 7 up: top of the viewport must show "line 0" — before
        // the coordinate fix this row came back blank.
        state.scroll_lines(7);
        let grid = state.grid_snapshot();
        let top: String = (0..7).map(|c| grid.get(0, c).character).collect();
        assert_eq!(top.trim_end(), "line 0");
        // The live cursor is below the scrolled viewport — hidden.
        assert_eq!(grid.cursor, None);
    }

    #[test]
    fn executing_phase_gates_on_osc_133() {
        let mut state = TerminalState::new(80, 24, 100, ResolvedTheme::default());
        assert!(!state.is_executing(), "no integration yet");
        state.process_bytes(b"\x1b]133;A\x07");
        assert!(!state.is_executing(), "at prompt");
        state.process_bytes(b"\x1b]133;C\x07");
        assert!(state.is_executing(), "command running");
        state.process_bytes(b"\x1b]133;D;0\x07");
        assert!(!state.is_executing(), "command finished");
    }

    /// The dogfood finding that forced content-anchored selection: a
    /// drag that autoscrolls must yield MORE text than one screen, and
    /// the selection must survive scrolling instead of being cleared.
    #[test]
    fn selection_survives_scrolling_and_spans_scrollback() {
        let mut state = TerminalState::new(80, 24, 1000, ResolvedTheme::default());
        for i in 0..60 {
            state.process_bytes(format!("line {}\r\n", i).as_bytes());
        }

        // The dogfood gesture: scroll up into history to the top of
        // the output first...
        state.scroll_lines(20);
        // ...anchor at the top visible row...
        state.start_selection(SelectMode::Char, 0, 0, false);
        // ...drag to the bottom edge...
        state.update_selection(79, 23, true);
        // ...and drag-autoscroll back toward the tail, re-pinning the
        // head to the bottom edge each step. The anchor stays glued to
        // its content up in history, so the selection grows.
        state.scroll_lines(-10);
        state.update_selection(79, 23, true);

        let text = state.selection_text().expect("selection has text");
        let lines: Vec<&str> = text.lines().collect();
        assert!(
            lines.len() > 24,
            "selection spans more than one screen: got {} lines",
            lines.len()
        );

        // Still selected after further scrolling — content-anchored.
        state.scroll_lines(5);
        assert!(state.selection_text().is_some());

        // And the snapshot carries SELECTION flags for visible cells.
        let grid = state.grid_snapshot();
        let any_selected = (0..grid.rows)
            .any(|r| (0..grid.cols).any(|c| grid.get(r, c).flags.contains(CellFlags::SELECTION)));
        assert!(any_selected, "highlight flags present in the viewport");
    }

    #[test]
    fn empty_click_selection_yields_no_text() {
        let mut state = TerminalState::new(80, 24, 100, ResolvedTheme::default());
        state.process_bytes(b"hello\r\n");
        state.start_selection(SelectMode::Char, 2, 0, false);
        // No drag — degenerate selection copies nothing.
        assert_eq!(state.selection_text(), None);
    }

    #[test]
    fn word_selection_snaps_to_boundaries() {
        let mut state = TerminalState::new(80, 24, 100, ResolvedTheme::default());
        state.process_bytes(b"alpha bravo charlie\r\n");
        // Double-click semantics: land mid-"bravo".
        state.start_selection(SelectMode::Word, 8, 0, false);
        assert_eq!(state.selection_text().as_deref(), Some("bravo"));
    }

    #[test]
    fn zero_scrollback_keeps_offset_pinned() {
        let mut state = TerminalState::new(80, 24, 0, ResolvedTheme::default());
        for i in 0..50 {
            state.process_bytes(format!("line {}\r\n", i).as_bytes());
        }
        state.scroll_lines(10);
        assert_eq!(state.display_offset(), 0, "no history to scroll into");
    }

    // ---- semantic stream (v0.3a) ----

    const A: &[u8] = b"\x1b]133;A\x1b\\";
    const B: &[u8] = b"\x1b]133;B\x1b\\";
    const C: &[u8] = b"\x1b]133;C\x1b\\";

    fn d(code: i32) -> Vec<u8> {
        format!("\x1b]133;D;{code}\x1b\\").into_bytes()
    }

    /// One full cycle: prompt on `prompt` (may hold a newline for a
    /// two-line prompt), echo `cmd`, `output` lines, exit `code`.
    fn cycle(state: &mut TerminalState, prompt: &str, cmd: &str, output: &[&str], code: i32) {
        state.process_bytes(A);
        state.process_bytes(prompt.as_bytes());
        state.process_bytes(B);
        state.process_bytes(cmd.as_bytes());
        state.process_bytes(b"\r\n");
        state.process_bytes(C);
        for line in output {
            state.process_bytes(format!("{line}\r\n").as_bytes());
        }
        state.process_bytes(&d(code));
    }

    #[test]
    fn run_end_splits_at_osc_terminators_and_escape_heads() {
        let bytes = b"ab\x1b]133;A\x1b\\cd\x07e";
        // "ab" ends before ESC.
        assert_eq!(run_end(bytes, 0), 2);
        // ESC ] 1 3 3 ; A — ends before the ST's ESC.
        assert_eq!(run_end(bytes, 2), 9);
        // ESC \\ — head ESC belongs to the run, backslash terminates.
        assert_eq!(run_end(bytes, 9), 11);
        // "cd" BEL — BEL is inclusive.
        assert_eq!(run_end(bytes, 11), 14);
        assert_eq!(run_end(bytes, 14), 15);
    }

    #[test]
    fn markers_record_the_row_and_column_they_fired_on() {
        let mut state = TerminalState::new(80, 24, 1000, ResolvedTheme::default());
        state.process_bytes(b"banner\r\n"); // row 0
        cycle(&mut state, "~/src ❯ ", "ls", &["a", "b", "c"], 0);
        let spans: Vec<Span> = state.tracker().spans().copied().collect();
        assert_eq!(spans.len(), 1);
        let span = spans[0];
        assert_eq!(span.prompt_start, 1, "A fired on row 1");
        assert_eq!(span.prompt_end, Some((1, 8)), "B after the 8-cell prompt");
        assert_eq!(span.output_start, Some(2), "C on the row after the echo");
        assert_eq!(span.end, Some(5), "D after three output rows");
        assert_eq!(span.exit_code, Some(0));
        assert_eq!(state.row_text(1), "~/src ❯ ls");
        assert_eq!(state.row_text(2), "a");
    }

    #[test]
    fn two_line_prompt_records_both_rows() {
        let mut state = TerminalState::new(80, 24, 1000, ResolvedTheme::default());
        state.process_bytes(A);
        state.process_bytes(b"~/src on main\r\n\xe2\x9d\xaf ");
        state.process_bytes(B);
        assert_eq!(state.tracker().live_prompt(), Some((0, 1)));
        let span = state.tracker().spans().next().copied().unwrap();
        assert_eq!(span.prompt_end, Some((1, 2)));
    }

    #[test]
    fn marker_rows_survive_scrolling_and_history_drops() {
        // Tiny cap so truncation actually happens; the slack above it
        // keeps drops countable.
        let mut state = TerminalState::new(80, 24, 40, ResolvedTheme::default());
        cycle(&mut state, "P1> ", "first", &["out1"], 0);
        let span = state.tracker().spans().next().copied().unwrap();
        assert_eq!(span.prompt_start, 0);

        // Push 30 more lines: the prompt row is now in history.
        for i in 0..30 {
            state.process_bytes(format!("filler {i}\r\n").as_bytes());
        }
        let history = state.term.grid().history_size();
        let line = state.tracker().line(history, span.prompt_start);
        assert!(line < 0, "prompt row scrolled into history: {line}");
        assert_eq!(state.row_text(line as i32), "P1> first");

        // Push far past the cap in one call: history is truncated back
        // to 40 in steps and the drops are counted exactly.
        for i in 0..500 {
            state.process_bytes(format!("more {i}\r\n").as_bytes());
        }
        assert!(state.term.grid().history_size() <= 40 + HISTORY_SLACK);
        assert!(state.tracker().dropped() > 0, "drops were counted");
        // The old span's rows are gone from the retained window.
        assert!(
            state
                .tracker()
                .spans()
                .all(|s| s.prompt_start >= state.tracker().dropped() || !s.is_closed()),
            "closed spans below the retained window are pruned"
        );
        // A fresh cycle at the tail still maps to real content.
        cycle(&mut state, "P2> ", "second", &["out2"], 3);
        let last = state.tracker().spans().last().copied().unwrap();
        let history = state.term.grid().history_size();
        let line = state.tracker().line(history, last.prompt_start);
        assert_eq!(state.row_text(line as i32), "P2> second");
        assert_eq!(state.row_text(line as i32 + 1), "out2");
    }

    #[test]
    fn history_cleared_keeps_screen_row_identity() {
        let mut state = TerminalState::new(80, 24, 1000, ResolvedTheme::default());
        cycle(&mut state, "P> ", "old", &["gone"], 0);
        for i in 0..30 {
            state.process_bytes(format!("filler {i}\r\n").as_bytes());
        }
        // Live prompt at the bottom of the screen.
        state.process_bytes(A);
        state.process_bytes(b"P> ");
        state.process_bytes(B);
        let (start, _) = state.tracker().live_prompt().unwrap();
        let history_before = state.term.grid().history_size();
        assert!(history_before > 0);
        let line_before = state.tracker().line(history_before, start);

        // macOS `clear` tail: wipe history only.
        state.process_bytes(b"\x1b[3J");
        assert_eq!(state.term.grid().history_size(), 0);
        let line_after = state.tracker().line(0, start);
        assert_eq!(line_after, line_before, "screen row keeps its identity");
        assert_eq!(state.row_text(line_after as i32), "P>");
        assert_eq!(state.tracker().spans().count(), 1, "history span pruned");
    }

    #[test]
    fn full_reset_drops_all_spans() {
        let mut state = TerminalState::new(80, 24, 1000, ResolvedTheme::default());
        cycle(&mut state, "P> ", "cmd", &["x"], 0);
        state.process_bytes(A);
        state.process_bytes(b"\x1bc");
        assert_eq!(state.tracker().spans().count(), 0);
    }

    #[test]
    fn row_resize_keeps_span_rows_column_resize_drops_them() {
        let mut state = TerminalState::new(80, 24, 1000, ResolvedTheme::default());
        for i in 0..40 {
            state.process_bytes(format!("filler {i}\r\n").as_bytes());
        }
        cycle(&mut state, "P> ", "cmd", &["out"], 0);
        let span = state.tracker().spans().next().copied().unwrap();

        // Grow rows: lines are pulled from history onto the screen.
        state.resize(80, 30);
        let history = state.term.grid().history_size();
        let line = state.tracker().line(history, span.prompt_start);
        assert_eq!(state.row_text(line as i32), "P> cmd", "after growing rows");
        assert_eq!(state.row_text(line as i32 + 1), "out");

        // Shrink rows: lines are pushed into history.
        state.resize(80, 12);
        let history = state.term.grid().history_size();
        let line = state.tracker().line(history, span.prompt_start);
        assert_eq!(
            state.row_text(line as i32),
            "P> cmd",
            "after shrinking rows"
        );

        // Column change reflows: spans go, tracking continues.
        state.resize(60, 12);
        assert_eq!(state.tracker().spans().count(), 0);
        cycle(&mut state, "P> ", "again", &["y"], 0);
        assert_eq!(state.tracker().spans().count(), 1);
    }

    #[test]
    fn live_prompt_is_pulled_off_the_bottom_when_screen_is_full() {
        let mut state = TerminalState::new(80, 24, 1000, ResolvedTheme::default());
        for i in 0..40 {
            state.process_bytes(format!("line {i}\r\n").as_bytes());
        }
        // Prompt lands on the bottom row (23).
        state.process_bytes(A);
        state.process_bytes(b"~ \xe2\x9d\xaf ");
        state.process_bytes(B);
        assert_eq!(
            state.tracker().live_prompt(),
            Some((
                {
                    let h = state.term.grid().history_size();
                    state.tracker().abs(h, 23)
                },
                {
                    let h = state.term.grid().history_size();
                    state.tracker().abs(h, 23)
                }
            ))
        );

        let grid = state.grid_snapshot();
        assert_eq!(grid.display_offset, 0, "user offset untouched");
        assert_eq!(
            TerminalState::grid_row_text(&grid, 23),
            "line 39",
            "last output line sits on the bottom row"
        );
        assert_eq!(
            grid.cursor, None,
            "prompt is below the viewport → default anchor"
        );
        // The offset was restored after the snapshot.
        assert_eq!(state.display_offset(), 0);
        // Mouse mapping folds the bump in: viewport row 23 → the row
        // holding "line 39".
        state.start_selection(SelectMode::Line, 0, 23, false);
        assert_eq!(state.selection_text().as_deref(), Some("line 39\n"));
    }

    /// Dogfood 2026-08-15: macOS `clear` sends only `\e[H\e[2J`, which
    /// pushes the screen into history instead of wiping it. The new
    /// prompt on row 0 must NOT pull that history back into view.
    #[test]
    fn clear_without_history_wipe_shows_a_blank_screen() {
        let mut state = TerminalState::new(80, 24, 1000, ResolvedTheme::default());
        for i in 0..40 {
            state.process_bytes(format!("line {i}\r\n").as_bytes());
        }
        state.process_bytes(b"\x1b[H\x1b[2J");
        assert!(
            state.term.grid().history_size() >= 40,
            "screen went into history"
        );
        state.process_bytes(A);
        state.process_bytes(b"~ >");
        state.process_bytes(B);
        let grid = state.grid_snapshot();
        for row in 0..24 {
            assert_eq!(
                TerminalState::grid_row_text(&grid, row),
                "",
                "row {row} must be blank after clear"
            );
        }
        assert_eq!(grid.cursor, None);
        assert_eq!(state.display_offset(), 0);

        // A short block after the clear anchors on its own last row and
        // still shows nothing from before the clear.
        state.process_bytes(b"ls\r\n");
        state.process_bytes(C);
        state.process_bytes(b"a\r\nb\r\n");
        state.process_bytes(&d(0));
        state.process_bytes(A);
        state.process_bytes(b"~ >");
        state.process_bytes(B);
        let grid = state.grid_snapshot();
        assert_eq!(TerminalState::grid_row_text(&grid, 0), "~ >ls");
        assert_eq!(TerminalState::grid_row_text(&grid, 2), "b");
        assert_eq!(
            TerminalState::grid_row_text(&grid, 3),
            "",
            "live prompt hidden"
        );
        assert_eq!(grid.cursor, Some((0, 2)), "anchor on the last output row");
        for row in 4..24 {
            assert_eq!(TerminalState::grid_row_text(&grid, row), "");
        }
    }

    /// Warp keeps the block that ran `clear`; so do we. Older content
    /// stays reachable by scrolling, with no blank screen in between.
    #[test]
    fn clear_keeps_its_own_block_and_scrolls_into_history_seamlessly() {
        let mut state = TerminalState::new(80, 24, 1000, ResolvedTheme::default());
        for i in 0..40 {
            state.process_bytes(format!("line {i}\r\n").as_bytes());
        }
        // `clear` cycle: header, then the clear itself as output.
        state.process_bytes(A);
        state.process_bytes(b"~ >");
        state.process_bytes(B);
        state.process_bytes(b"clear\r\n");
        state.process_bytes(C);
        // The exact bytes macOS `clear` emits: wipe history, home,
        // clear screen (which scrolls the visible rows INTO history).
        state.process_bytes(b"\x1b[3J\x1b[H\x1b[2J");
        state.process_bytes(&d(0));
        state.process_bytes(A);
        state.process_bytes(b"~ >");
        state.process_bytes(B);

        let grid = state.grid_snapshot();
        assert_eq!(
            TerminalState::grid_row_text(&grid, 0),
            "~ >clear",
            "the clear block is pulled back into view"
        );
        assert_eq!(grid.cursor, Some((0, 0)), "…and anchors at the bottom");
        for row in 1..24 {
            assert_eq!(TerminalState::grid_row_text(&grid, row), "", "row {row}");
        }

        // Scroll up: scrolled mode is a plain window — old content
        // enters at the TOP edge and slides DOWN one row per notch
        // (no bottom anchor; anchored scrolling read as "a cover being
        // lifted"). The clear block rides down with it.
        state.scroll_lines(1);
        let grid = state.grid_snapshot();
        assert_eq!(TerminalState::grid_row_text(&grid, 0), "line 39");
        assert_eq!(TerminalState::grid_row_text(&grid, 1), "~ >clear");
        assert_eq!(grid.cursor, None, "no anchor while scrolled");
        state.scroll_lines(1);
        let grid = state.grid_snapshot();
        assert_eq!(TerminalState::grid_row_text(&grid, 0), "line 38");
        assert_eq!(TerminalState::grid_row_text(&grid, 1), "line 39");
        assert_eq!(TerminalState::grid_row_text(&grid, 2), "~ >clear");
        assert_eq!(grid.cursor, None);
        state.scroll_to_bottom();

        // The next command stacks under the clear block.
        state.process_bytes(b"ls\r\n");
        state.process_bytes(C);
        state.process_bytes(b"a\r\n");
        state.process_bytes(&d(0));
        state.process_bytes(A);
        state.process_bytes(b"~ >");
        state.process_bytes(B);
        let grid = state.grid_snapshot();
        assert_eq!(TerminalState::grid_row_text(&grid, 0), "~ >clear");
        assert_eq!(TerminalState::grid_row_text(&grid, 1), "~ >ls");
        assert_eq!(TerminalState::grid_row_text(&grid, 2), "a");
        assert_eq!(grid.cursor, Some((0, 2)));
    }

    /// A command that clears after real output (watch loops) starts the
    /// view fresh at the clear instead of dragging the previous run in.
    #[test]
    fn chatty_command_that_clears_starts_a_fresh_view() {
        let mut state = TerminalState::new(80, 24, 1000, ResolvedTheme::default());
        state.process_bytes(A);
        state.process_bytes(b"~ >");
        state.process_bytes(B);
        state.process_bytes(b"loop\r\n");
        state.process_bytes(C);
        for i in 0..10 {
            state.process_bytes(format!("run1 {i}\r\n").as_bytes());
        }
        state.process_bytes(b"\x1b[3J\x1b[H\x1b[2J");
        state.process_bytes(b"run2 0\r\nrun2 1\r\n");
        state.process_bytes(&d(0));
        state.process_bytes(A);
        state.process_bytes(b"~ >");
        state.process_bytes(B);
        let grid = state.grid_snapshot();
        assert_eq!(TerminalState::grid_row_text(&grid, 0), "run2 0");
        assert_eq!(TerminalState::grid_row_text(&grid, 1), "run2 1");
        assert_eq!(grid.cursor, Some((0, 1)));
        assert_eq!(TerminalState::grid_row_text(&grid, 2), "");
    }

    // ---- block navigation ----

    #[test]
    fn block_command_at_row_reads_the_echo_and_scroll_to_block_steps() {
        let mut state = TerminalState::new(80, 24, 1000, ResolvedTheme::default());
        cycle(&mut state, "~ >", "echo one", &["one"], 0);
        for i in 0..30 {
            state.process_bytes(format!("filler {i}\r\n").as_bytes());
        }
        // Wait — filler here is inside no span; give it a real cycle.
        cycle(&mut state, "~ >", "ls -la /tmp", &["a", "b"], 0);
        state.process_bytes(A);
        state.process_bytes(b"~ >");
        state.process_bytes(B);

        // At the tail: the `ls -la /tmp` header is visible; find its row.
        let grid = state.grid_snapshot();
        let header_row = (0..grid.rows)
            .find(|&r| TerminalState::grid_row_text(&grid, r) == "~ >ls -la /tmp")
            .expect("ls header visible");
        assert_eq!(
            state.block_command_at_row(header_row).as_deref(),
            Some("ls -la /tmp")
        );
        assert_eq!(
            state.block_command_at_row(header_row + 1),
            None,
            "output row"
        );

        // Previous block: the `ls` header is on the live screen (can't
        // be scrolled to the top), so the first hop is `echo one`.
        assert!(state.scroll_to_block(-1), "there is a previous block");
        let grid = state.grid_snapshot();
        assert_eq!(TerminalState::grid_row_text(&grid, 0), "~ >echo one");
        assert!(!state.scroll_to_block(-1), "no block above the first");
        // Next: the `ls` header lives on the live screen, so the closest
        // the viewport can get is the tail (offset 0) with it visible.
        assert!(state.scroll_to_block(1));
        assert_eq!(state.display_offset(), 0);
        let grid = state.grid_snapshot();
        assert!((0..grid.rows).any(|r| TerminalState::grid_row_text(&grid, r) == "~ >ls -la /tmp"));
    }

    // ---- search (F14) ----

    #[test]
    fn search_focuses_newest_match_first_and_cycles() {
        let mut state = TerminalState::new(80, 24, 1000, ResolvedTheme::default());
        for i in 0..60 {
            let word = if i % 20 == 5 { "needle" } else { "hay" };
            state.process_bytes(format!("line {i} {word}\r\n").as_bytes());
        }
        // Matches at lines 5, 25, 45. Live screen shows 37..60.
        assert!(state.search_set("NEEDLE"), "case-insensitive literal");
        let (pos, total) = state.search_status().unwrap();
        assert_eq!(total, 3);
        assert_eq!(pos, Some(3), "newest match focused first");
        assert_eq!(state.display_offset(), 0, "already visible — no scroll");

        state.search_prev();
        let (pos, _) = state.search_status().unwrap();
        assert_eq!(pos, Some(2));
        assert!(
            state.display_offset() > 0,
            "scrolled to bring line 25 into view"
        );
        let grid = state.grid_snapshot();
        let focused_rows: Vec<usize> = (0..grid.rows)
            .filter(|&r| {
                (0..grid.cols).any(|c| grid.get(r, c).flags.contains(CellFlags::SEARCH_FOCUS))
            })
            .collect();
        assert_eq!(focused_rows.len(), 1, "one focused row visible");
        let r = focused_rows[0];
        assert!(TerminalState::grid_row_text(&grid, r).ends_with("needle"));
        // The focused cells are exactly the word.
        let flagged: String = (0..grid.cols)
            .filter(|&c| grid.get(r, c).flags.contains(CellFlags::SEARCH_FOCUS))
            .map(|c| grid.get(r, c).character)
            .collect();
        assert_eq!(flagged, "needle");

        state.search_prev();
        assert_eq!(state.search_status().unwrap().0, Some(1));
        state.search_prev();
        assert_eq!(state.search_status().unwrap().0, Some(3), "wraps to newest");
        state.search_next();
        assert_eq!(state.search_status().unwrap().0, Some(1), "wraps to oldest");

        state.search_clear();
        assert!(!state.search_active());
        let grid = state.grid_snapshot();
        assert!(
            !(0..grid.rows)
                .any(|r| (0..grid.cols)
                    .any(|c| grid.get(r, c).flags.contains(CellFlags::SEARCH_MATCH))),
            "no flags after clear"
        );
    }

    #[test]
    fn search_with_no_match_and_regex_metachars() {
        let mut state = TerminalState::new(80, 24, 1000, ResolvedTheme::default());
        state.process_bytes(b"price is $5.00 (approx)\r\n");
        assert!(
            state.search_set("$5.00 (approx)"),
            "metacharacters matched literally"
        );
        assert_eq!(state.search_status().unwrap(), (Some(1), 1));
        assert!(state.search_set("zzz"));
        assert_eq!(state.search_status().unwrap(), (None, 0));
        assert!(!state.search_set(""), "empty query clears");
        assert!(!state.search_active());
    }

    #[test]
    fn hidden_cursor_is_reported() {
        let mut state = TerminalState::new(80, 24, 100, ResolvedTheme::default());
        state.process_bytes(b"x");
        assert!(!state.grid_snapshot().cursor_hidden);
        state.process_bytes(b"\x1b[?25l");
        assert!(state.grid_snapshot().cursor_hidden);
        state.process_bytes(b"\x1b[?25h");
        assert!(!state.grid_snapshot().cursor_hidden);
    }

    #[test]
    fn live_prompt_in_short_session_anchors_above_it() {
        let mut state = TerminalState::new(80, 24, 1000, ResolvedTheme::default());
        state.process_bytes(b"one\r\ntwo\r\n");
        state.process_bytes(A);
        state.process_bytes(b"~ >");
        state.process_bytes(B);
        let grid = state.grid_snapshot();
        assert_eq!(TerminalState::grid_row_text(&grid, 1), "two");
        assert_eq!(
            TerminalState::grid_row_text(&grid, 2),
            "",
            "prompt row blanked"
        );
        assert_eq!(grid.cursor, Some((0, 1)), "anchor on the last output row");
    }

    #[test]
    fn first_prompt_of_a_session_hides_entirely() {
        let mut state = TerminalState::new(80, 24, 1000, ResolvedTheme::default());
        state.process_bytes(A);
        state.process_bytes(b"~ >");
        state.process_bytes(B);
        let grid = state.grid_snapshot();
        assert_eq!(TerminalState::grid_row_text(&grid, 0), "");
        assert_eq!(grid.cursor, None);
    }

    #[test]
    fn prompt_reappears_as_header_once_command_runs() {
        let mut state = TerminalState::new(80, 24, 1000, ResolvedTheme::default());
        state.process_bytes(A);
        state.process_bytes(b"~ >");
        state.process_bytes(B);
        state.process_bytes(b"sleep 1\r\n");
        state.process_bytes(C);
        let grid = state.grid_snapshot();
        assert_eq!(TerminalState::grid_row_text(&grid, 0), "~ >sleep 1");
        let theme = ResolvedTheme::default();
        assert_eq!(grid.get(0, 0).bg, theme.block_header, "header tinted");
        assert!(grid.get(0, 3).flags.contains(CellFlags::BOLD), "echo bold");
        assert!(
            !grid.get(0, 0).flags.contains(CellFlags::BOLD),
            "prompt not bold"
        );
        assert_eq!(
            grid.cursor,
            Some((0, 1)),
            "real cursor row anchors while running"
        );
    }

    #[test]
    fn failed_command_gets_red_header_and_exit_label() {
        let mut state = TerminalState::new(40, 24, 1000, ResolvedTheme::default());
        cycle(&mut state, "P> ", "false", &[], 1);
        cycle(&mut state, "P> ", "true", &["ok"], 0);
        let theme = ResolvedTheme::default();
        let grid = state.grid_snapshot();
        // Row 0: failed header.
        assert_eq!(grid.get(0, 0).bg, theme.block_failed);
        let tail: String = (36..40).map(|c| grid.get(0, c).character).collect();
        assert_eq!(tail, "✘ 1 ");
        assert_eq!(grid.get(0, 36).fg, theme.ansi(1));
        // Row 1: successful header, no label.
        assert_eq!(grid.get(1, 0).bg, theme.block_header);
        let tail: String = (36..40).map(|c| grid.get(1, c).character).collect();
        assert_eq!(tail, "    ");
        // Row 2: output, untinted.
        assert_eq!(TerminalState::grid_row_text(&grid, 2), "ok");
        assert_eq!(grid.get(2, 0).bg, theme.background);
    }

    #[test]
    fn exit_label_yields_to_a_right_prompt() {
        let mut state = TerminalState::new(20, 24, 1000, ResolvedTheme::default());
        // Prompt fills the row to the right edge (RPROMPT-style).
        cycle(&mut state, "P>             14:02", "", &[], 2);
        let grid = state.grid_snapshot();
        let tail: String = (15..20).map(|c| grid.get(0, c).character).collect();
        assert_eq!(tail, "14:02", "no room → no label");
        assert_eq!(grid.get(0, 0).bg, ResolvedTheme::default().block_failed);
    }

    #[test]
    fn blocks_disabled_leaves_the_stream_plain() {
        let mut state = TerminalState::new(80, 24, 1000, ResolvedTheme::default());
        state.set_blocks_enabled(false);
        cycle(&mut state, "P> ", "false", &[], 1);
        state.process_bytes(A);
        state.process_bytes(b"P> ");
        state.process_bytes(B);
        let grid = state.grid_snapshot();
        let theme = ResolvedTheme::default();
        assert_eq!(grid.get(0, 0).bg, theme.background);
        assert_eq!(
            TerminalState::grid_row_text(&grid, 1),
            "P>",
            "live prompt visible"
        );
        assert_eq!(grid.cursor, Some((3, 1)));
    }

    #[test]
    fn alt_screen_ignores_markers_and_hiding() {
        let mut state = TerminalState::new(80, 24, 1000, ResolvedTheme::default());
        state.process_bytes(A);
        state.process_bytes(b"P> ");
        state.process_bytes(B);
        state.process_bytes(b"vim\r\n");
        state.process_bytes(C);
        state.process_bytes(b"\x1b[?1049h"); // enter alt screen
        state.process_bytes(A); // a stray marker inside the program
        assert!(state.is_alt_screen());
        assert_eq!(
            state.tracker().spans().count(),
            1,
            "no new span from alt screen"
        );
        let grid = state.grid_snapshot();
        assert!(grid.cursor.is_some(), "alt screen keeps the real cursor");
        state.process_bytes(b"\x1b[?1049l");
        assert!(!state.is_alt_screen());
        state.process_bytes(&d(0));
        assert!(state.tracker().spans().next().unwrap().is_closed());
    }

    #[test]
    fn scrolled_up_view_still_hides_live_prompt_and_keeps_offset() {
        let mut state = TerminalState::new(80, 24, 1000, ResolvedTheme::default());
        for i in 0..60 {
            state.process_bytes(format!("line {i}\r\n").as_bytes());
        }
        state.process_bytes(A);
        state.process_bytes(b"P> ");
        state.process_bytes(B);
        state.scroll_lines(10);
        let grid = state.grid_snapshot();
        assert_eq!(grid.display_offset, 10);
        // Bump 1 on top of the user's 10: 61 content rows, viewport is
        // rows 26..=49 → the bottom row shows "line 49".
        assert_eq!(TerminalState::grid_row_text(&grid, 23), "line 49");
        assert_eq!(state.display_offset(), 10, "user offset restored");
    }
}
