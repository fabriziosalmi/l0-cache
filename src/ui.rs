//! Terminal UI primitives shared by `--stats` and `--doctor`.
//!
//! Two concerns live here:
//!   1. A color guard — ANSI is emitted only when it makes sense (a real TTY,
//!      `NO_COLOR` unset, `TERM` not `dumb`), so piping `--stats` to a file or
//!      pager yields clean text instead of raw escape codes.
//!   2. Box-drawing primitives with *visible-width* accounting, so colored
//!      segments (which carry zero-width ANSI) still align inside a fixed frame.

use std::io::IsTerminal;

// Inner content width of a boxed row (the cells between "│ " and " │").
pub const INNER: usize = 62;
// Number of horizontal glyphs between two box corners.
const RULE: usize = INNER + 2; // 64

/// Whether ANSI color should be emitted on stdout.
///
/// Precedence: `FORCE_COLOR`/`CLICOLOR_FORCE` force it on (handy for CI captures
/// and screenshots); otherwise `NO_COLOR` (any value, per https://no-color.org)
/// or `TERM=dumb` force it off; otherwise it follows whether stdout is a TTY.
pub fn color_enabled() -> bool {
    if std::env::var_os("FORCE_COLOR").is_some() || std::env::var_os("CLICOLOR_FORCE").is_some() {
        return true;
    }
    if std::env::var_os("NO_COLOR").is_some() {
        return false;
    }
    if std::env::var("TERM").as_deref() == Ok("dumb") {
        return false;
    }
    std::io::stdout().is_terminal()
}

/// Visible width of a plain (ANSI-free) string. Good enough for our content:
/// box/block glyphs are all single-width, and command names are truncated by
/// `char` count elsewhere, so a codepoint count matches columns on screen.
fn vis_len(s: &str) -> usize {
    s.chars().count()
}

/// Carries the one-time color decision plus the palette.
#[derive(Clone, Copy)]
pub struct Ui {
    pub color: bool,
}

impl Ui {
    pub fn new() -> Self {
        Ui {
            color: color_enabled(),
        }
    }

    /// Wrap `s` in SGR code(s), or return it untouched when color is off.
    pub fn paint(&self, code: &str, s: &str) -> String {
        if self.color {
            format!("\x1b[{}m{}\x1b[0m", code, s)
        } else {
            s.to_string()
        }
    }

    pub fn dim(&self, s: &str) -> String {
        self.paint("38;5;245", s)
    }
    pub fn faint(&self, s: &str) -> String {
        self.paint("38;5;238", s)
    }
    pub fn bold(&self, s: &str) -> String {
        self.paint("1", s)
    }
    pub fn cyan(&self, s: &str) -> String {
        self.paint("1;36", s)
    }
    pub fn green(&self, s: &str) -> String {
        self.paint("32", s)
    }
    pub fn yellow(&self, s: &str) -> String {
        self.paint("33", s)
    }
    pub fn red(&self, s: &str) -> String {
        self.paint("31", s)
    }

    /// Color a percentage red → orange → green by magnitude.
    pub fn pct_code(&self, pct: f64) -> &'static str {
        match pct {
            p if p > 80.0 => "38;5;46",
            p if p > 40.0 => "38;5;214",
            _ => "38;5;196",
        }
    }

    // ── Box drawing ─────────────────────────────────────────────────────────

    fn border(&self, s: &str) -> String {
        self.faint(s)
    }

    /// Top border with an embedded title (left) and optional right-side label,
    /// e.g. `┌─ l0-cache TELEMETRY ─────────────── last 7d ─┐`.
    pub fn box_top(&self, title: &str, right: &str) -> String {
        // Fixed (non-dash) glyphs on this line, by branch:
        //   empty right: "┌─ " + title + " " + "┐"                → 5 + title
        //   with  right: "┌─ " + title + "  " + right + " ─┐"     → 8 + title + right
        let total = RULE + 2; // full visible width, corners included
        let fixed = if right.is_empty() {
            5 + vis_len(title)
        } else {
            8 + vis_len(title) + vis_len(right)
        };
        let dashes = total.saturating_sub(fixed);
        let mut out = String::new();
        out.push_str(&self.border("┌─ "));
        out.push_str(&self.cyan(title));
        out.push(' ');
        out.push_str(&self.border(&"─".repeat(dashes)));
        if right.is_empty() {
            out.push_str(&self.border("┐"));
        } else {
            out.push(' ');
            out.push_str(&self.dim(right));
            out.push_str(&self.border(" ─┐"));
        }
        out
    }

    pub fn box_div(&self) -> String {
        self.border(&format!("├{}┤", "─".repeat(RULE)))
    }

    pub fn box_bottom(&self) -> String {
        self.border(&format!("└{}┘", "─".repeat(RULE)))
    }

    /// Wrap a built [`Line`] in side borders, padding it to the inner width.
    pub fn box_row(&self, mut line: Line) -> String {
        line.pad(INNER);
        format!("{} {} {}", self.border("│"), line.done(), self.border("│"))
    }

    pub fn line(&self) -> Line {
        Line::new(self.color)
    }

    // ── Diagnostic / list helpers (shared with --doctor) ─────────────────────

    /// A numbered section header, e.g. `● 1. Binary & PATH`.
    pub fn section(&self, title: &str) -> String {
        format!("{} {}", self.cyan("●"), self.bold(title))
    }

    /// An indented key/value line with a fixed-width, dim key.
    pub fn field(&self, key: &str, val: &str) -> String {
        format!("  {} {}", self.dim(&format!("{:<18}", key)), val)
    }

    pub fn ok(&self, msg: &str) -> String {
        format!("  {} {}", self.green("✔"), msg)
    }
    pub fn warn(&self, msg: &str) -> String {
        format!("  {} {}", self.yellow("⚠"), msg)
    }
    pub fn err(&self, msg: &str) -> String {
        format!("  {} {}", self.red("✗"), msg)
    }

    /// A dim, deeper-indented follow-up line under a status.
    pub fn hint(&self, msg: &str) -> String {
        format!("    {} {}", self.dim("↳"), self.dim(msg))
    }
}

impl Default for Ui {
    fn default() -> Self {
        Self::new()
    }
}

/// Accumulates text segments while tracking *visible* width, so a row built from
/// a mix of plain text, colored spans, and pre-rendered fragments can be padded
/// to an exact column count without ANSI throwing the math off.
pub struct Line {
    buf: String,
    vis: usize,
    color: bool,
}

impl Line {
    fn new(color: bool) -> Self {
        Line {
            buf: String::new(),
            vis: 0,
            color,
        }
    }

    /// Append plain, uncolored text.
    pub fn text(&mut self, s: &str) -> &mut Self {
        self.buf.push_str(s);
        self.vis += vis_len(s);
        self
    }

    /// Append `s` wrapped in SGR `code` (or plain when color is off). `s` must
    /// be ANSI-free; its codepoint count is added to the visible width.
    pub fn paint(&mut self, code: &str, s: &str) -> &mut Self {
        if self.color {
            self.buf.push_str("\x1b[");
            self.buf.push_str(code);
            self.buf.push('m');
            self.buf.push_str(s);
            self.buf.push_str("\x1b[0m");
        } else {
            self.buf.push_str(s);
        }
        self.vis += vis_len(s);
        self
    }

    /// Append a pre-rendered fragment (possibly already containing ANSI) whose
    /// visible width the caller knows. Used for composite cells like meters.
    pub fn raw(&mut self, s: &str, visible: usize) -> &mut Self {
        self.buf.push_str(s);
        self.vis += visible;
        self
    }

    /// Right-pad with spaces up to `width` visible columns (no-op if already wider).
    pub fn pad(&mut self, width: usize) -> &mut Self {
        if self.vis < width {
            let n = width - self.vis;
            self.buf.push_str(&" ".repeat(n));
            self.vis += n;
        }
        self
    }

    fn done(self) -> String {
        self.buf
    }
}

/// A proportional meter of `width` cells filled to `pct` (0–100), colored by
/// magnitude. Returns `(rendered, width)` so it can be fed to [`Line::raw`].
pub fn meter(ui: &Ui, pct: f64, width: usize) -> (String, usize) {
    let filled = ((pct / 100.0) * width as f64).round() as usize;
    let filled = filled.min(width);
    let empty = width - filled;
    let bar = format!(
        "{}{}",
        ui.paint(ui.pct_code(pct), &"█".repeat(filled)),
        ui.faint(&"░".repeat(empty))
    );
    (bar, width)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mono() -> Ui {
        Ui { color: false }
    }

    fn visible_cols(s: &str) -> usize {
        let stripped = strip_ansi_escapes::strip(s.as_bytes());
        String::from_utf8_lossy(&stripped).chars().count()
    }

    #[test]
    fn colored_row_aligns_to_box_width() {
        let ui = Ui { color: true };
        let mut l = ui.line();
        l.paint("31", "abc").text("d");
        let row = ui.box_row(l);
        // The visible width matches an all-plain box row, even though ANSI made
        // the byte length larger.
        assert_eq!(visible_cols(&row), INNER + 4);
        assert!(row.len() > visible_cols(&row));
    }

    #[test]
    fn mono_line_has_no_escapes() {
        let ui = mono();
        let mut l = ui.line();
        l.paint("31", "abc").pad(10);
        let s = l.done();
        assert!(!s.contains('\x1b'));
        assert_eq!(s, "abc       ");
    }

    #[test]
    fn box_borders_share_one_width() {
        let ui = mono();
        let top = ui.box_top("TITLE", "right");
        let div = ui.box_div();
        let bottom = ui.box_bottom();
        let row = ui.box_row({
            let mut l = ui.line();
            l.text("x");
            l
        });
        for line in [&top, &div, &bottom, &row] {
            assert_eq!(line.chars().count(), INNER + 4, "line: {line}");
        }
    }

    #[test]
    fn meter_clamps_and_sizes() {
        let ui = mono();
        let (s, w) = meter(&ui, 150.0, 10);
        assert_eq!(w, 10);
        assert_eq!(s.chars().filter(|c| *c == '█').count(), 10);
    }
}
