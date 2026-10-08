//! Port of `packages/astroshot-review/src/ui/settings.tsx`.
//!
//! The Settings pane: watched folders, graphics capabilities, ffmpeg and the
//! harness layout. `<SettingsPane roots width height>` is the
//! [`SettingsPane`] widget over the `Rect` it is given; the `useServices()`
//! reads (`capabilities`, `ffmpeg`, `rootsSource`, `version`) are borrowed
//! fields, filled from `&AppServices` by [`SettingsPane::new`]. `os.homedir()`
//! and `indexCachePath()` are read once at construction so tests can inject
//! them with [`SettingsPane::with_paths`].
//!
//! The Ink column clips at its height (`overflow="hidden"`); rows are laid out
//! top to bottom and rows past the bottom edge are not drawn.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Span;
use ratatui::widgets::Widget;

use crate::data::index_cache::default_index_cache_path;
use crate::terminal::probe::{CellSource, GraphicsProtocol, TerminalCapabilities};
use crate::ui::chrome::{MetaRow, rule, section_label, wrap_words};
use crate::ui::context::{AppServices, RootsSource};
use crate::ui::theme::THEME;
use crate::ui::{put_spans, text_width};
use crate::video::ffmpeg::FfmpegInfo;
use astroshot_engine::review_data::paths::abbreviate_home;

pub struct SettingsPane<'a> {
    pub roots: &'a [String],
    pub capabilities: &'a TerminalCapabilities,
    pub ffmpeg: &'a FfmpegInfo,
    pub roots_source: RootsSource,
    pub version: &'a str,
    pub command: &'a str,
    pub home: String,
    pub index_path: String,
}

impl<'a> SettingsPane<'a> {
    /// Reads the home directory and the index cache path from the environment.
    pub fn new(roots: &'a [String], services: &'a AppServices) -> Self {
        let home = dirs::home_dir()
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_default();
        let index_path = default_index_cache_path().to_string_lossy().into_owned();
        Self::with_paths(roots, services, home, index_path)
    }

    pub fn with_paths(
        roots: &'a [String],
        services: &'a AppServices,
        home: String,
        index_path: String,
    ) -> Self {
        Self {
            roots,
            capabilities: &services.capabilities,
            ffmpeg: &services.ffmpeg,
            roots_source: services.roots_source,
            version: &services.version,
            command: &services.command,
            home,
            index_path,
        }
    }
}

fn cell_source_label(source: CellSource) -> &'static str {
    match source {
        CellSource::Query => "query",
        CellSource::Env => "env",
        CellSource::Fallback => "fallback",
    }
}

/// Ink `wrap="truncate"`: cut the end and mark it with `…`.
fn truncate_end(text: &str, width: usize) -> String {
    if text_width(text) <= width {
        return text.to_string();
    }
    if width == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut used = 1; // the ellipsis
    for c in text.chars() {
        let w = text_width(c.encode_utf8(&mut [0; 4]));
        if used + w > width {
            break;
        }
        used += w;
        out.push(c);
    }
    out.push('…');
    out
}

/// Top-to-bottom row writer clipped to the pane (Ink `overflow="hidden"`).
struct Rows<'b> {
    buf: &'b mut Buffer,
    content: Rect,
    y: u16,
}

impl Rows<'_> {
    fn visible(&self) -> bool {
        self.y < self.content.bottom()
    }

    fn line(&mut self, spans: &[Span<'_>]) {
        if self.visible() {
            put_spans(self.buf, self.content, self.content.x, self.y, spans);
        }
        self.y += 1;
    }

    fn muted_wrapped(&mut self, text: &str) {
        let style = Style::new().fg(THEME.muted);
        for piece in wrap_words(text, self.content.width as usize) {
            self.line(&[Span::styled(piece, style)]);
        }
    }

    fn meta(&mut self, label: &str, value: &str) {
        if self.visible() {
            let area = Rect::new(self.content.x, self.y, self.content.width, 1);
            MetaRow { label, value }.render(area, self.buf);
        }
        self.y += 1;
    }

    fn rule(&mut self) {
        let width = self.content.width as usize;
        self.line(&[rule(width, None)]);
    }
}

impl Widget for SettingsPane<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        // paddingX={1}
        let inner = area.width.saturating_sub(2);
        let content = Rect::new((area.x + 1).min(area.right()), area.y, inner, area.height);
        let mut rows = Rows {
            buf,
            content,
            y: content.y,
        };

        // Header: `‹ Stream` left, version right (justify space-between).
        let right = format!("{} {}", self.command, self.version);
        let left_end = put_spans(
            rows.buf,
            content,
            content.x,
            content.y,
            &[Span::styled("‹ Stream", Style::new().fg(THEME.blue))],
        );
        let right_x = (usize::from(content.right()))
            .saturating_sub(text_width(&right))
            .max(usize::from(left_end)) as u16;
        put_spans(
            rows.buf,
            content,
            right_x,
            content.y,
            &[Span::styled(right, Style::new().fg(THEME.muted))],
        );
        rows.y += 1;

        rows.line(&[section_label("WATCHED FOLDERS")]);
        let source_label = match self.roots_source {
            RootsSource::App => "from the Astroshots app preferences",
            RootsSource::Cli => "from --root",
            RootsSource::Cwd => "current directory",
        };
        if self.roots.is_empty() {
            rows.muted_wrapped(
                "Pick the directories that contain your projects. Nothing is watched until you choose at least one.",
            );
            rows.line(&[Span::styled(
                format!("No folders yet · {} --root ~/projects", self.command),
                Style::new().fg(THEME.faint),
            )]);
        } else {
            rows.muted_wrapped(&format!(
                "Every worktree below any of these folders streams into one feed ({source_label})."
            ));
            for root in self.roots {
                let shown = truncate_end(
                    &abbreviate_home(root, &self.home),
                    usize::from(inner).saturating_sub(2),
                );
                rows.line(&[
                    Span::styled("● ", Style::new().fg(THEME.green)),
                    Span::raw(shown),
                ]);
            }
        }
        rows.rule();

        let caps = self.capabilities;
        let reason = caps.reason.as_deref();
        let images = match caps.graphics {
            GraphicsProtocol::Kitty => "Kitty graphics (pixel-perfect)".to_string(),
            GraphicsProtocol::Herdr => "herdr pane graphics (pixel-perfect)".to_string(),
            GraphicsProtocol::Halfblocks => format!("half-block text · {}", reason.unwrap_or(""))
                .trim()
                .to_string(),
            GraphicsProtocol::None => format!("off · {}", reason.unwrap_or("unsupported")),
        };
        let mut session = vec![if caps.inside_ssh { "ssh" } else { "local" }];
        if caps.inside_mosh {
            session.push("mosh");
        }
        if caps.inside_herdr {
            session.push("herdr");
        }
        if caps.inside_tmux {
            session.push("tmux");
        }
        let ffmpeg = match (&self.ffmpeg.ffmpeg, &self.ffmpeg.version) {
            (Some(path), Some(version)) if !version.is_empty() => format!("{path} ({version})"),
            (Some(path), _) => path.clone(),
            (None, _) => "not found · brew install ffmpeg".to_string(),
        };

        rows.line(&[section_label("GRAPHICS")]);
        rows.meta("Images", &images);
        rows.meta(
            "Cell",
            &format!(
                "{}×{} px ({})",
                caps.cell_width,
                caps.cell_height,
                cell_source_label(caps.cell_source)
            ),
        );
        rows.meta(
            "Transport",
            if caps.file_medium {
                "file path (local)"
            } else {
                "inline bytes"
            },
        );
        rows.meta("Session", &session.join(" · "));
        rows.rule();

        rows.line(&[section_label("MOVIES")]);
        rows.meta("ffmpeg", &ffmpeg);
        rows.muted_wrapped(
            "Movies play in the tray only with Kitty graphics. Otherwise the poster shows and O opens the file in your default player.",
        );
        rows.rule();

        rows.line(&[section_label("HARNESS LAYOUT")]);
        for text in [
            "Write frames here so Astroshots can find them:",
            "  .astroshot/<feature>/",
            "    manifest.json",
            "    0001-slug.png",
        ] {
            rows.line(&[Span::styled(text, Style::new().fg(THEME.muted))]);
        }
        rows.rule();
        rows.meta("Index", &abbreviate_home(&self.index_path, &self.home));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::context::test_support::{FakeService, services};
    use crate::ui::testing::{fg_at, modifier_at, render, rows};
    use ratatui::style::{Color, Modifier};

    fn draw(roots: &[String], services: &AppServices, width: u16, height: u16) -> Buffer {
        render(
            SettingsPane::with_paths(
                roots,
                services,
                "/Users/me".to_string(),
                "/Users/me/.cache/astroshot-review/index.json".to_string(),
            ),
            width,
            height,
        )
    }

    #[tokio::test]
    async fn empty_roots_show_the_pick_a_folder_prompt() {
        let mut services = services(GraphicsProtocol::None, FakeService::new(false));
        services.version = "1.2.3".into();
        services.capabilities.reason = Some("not a kitty terminal".into());
        let buf = draw(&[], &services, 60, 30);
        assert_eq!(
            rows(&buf),
            vec![
                " ‹ Stream                            astroshot review 1.2.3",
                " WATCHED FOLDERS",
                " Pick the directories that contain your projects. Nothing",
                " is watched until you choose at least one.",
                " No folders yet · astroshot review --root ~/projects",
                &format!(" {}", "─".repeat(58)),
                " GRAPHICS",
                " Images    off · not a kitty terminal",
                " Cell      10×20 px (env)",
                " Transport inline bytes",
                " Session   local",
                &format!(" {}", "─".repeat(58)),
                " MOVIES",
                " ffmpeg    not found · brew install ffmpeg",
                " Movies play in the tray only with Kitty graphics.",
                " Otherwise the poster shows and O opens the file in your",
                " default player.",
                &format!(" {}", "─".repeat(58)),
                " HARNESS LAYOUT",
                " Write frames here so Astroshots can find them:",
                "   .astroshot/<feature>/",
                "     manifest.json",
                "     0001-slug.png",
                &format!(" {}", "─".repeat(58)),
                " Index     ~/.cache/astroshot-review/index.json",
                "",
                "",
                "",
                "",
                "",
            ]
        );
        assert_eq!(fg_at(&buf, 1, 1), THEME.muted);
        assert!(modifier_at(&buf, 1, 1).contains(Modifier::BOLD));
        assert_eq!(fg_at(&buf, 1, 0), THEME.blue);
        assert_eq!(fg_at(&buf, 1, 4), THEME.faint);
        assert_eq!(fg_at(&buf, 1, 5), THEME.faint);
        assert_eq!(fg_at(&buf, 11, 7), Color::Reset);
    }

    #[tokio::test]
    async fn roots_are_listed_with_home_abbreviated_and_the_source_label() {
        let mut services = services(GraphicsProtocol::Kitty, FakeService::new(false));
        services.roots_source = RootsSource::App;
        services.ffmpeg.ffmpeg = Some("/opt/homebrew/bin/ffmpeg".into());
        services.ffmpeg.version = Some("7.1".into());
        services.capabilities.file_medium = true;
        services.capabilities.inside_ssh = true;
        services.capabilities.inside_tmux = true;
        let roots = vec![
            "/Users/me/projects".to_string(),
            "/work/a-very-long-directory-name-that-cannot-possibly-fit".to_string(),
        ];
        let buf = draw(&roots, &services, 48, 24);
        let lines = rows(&buf);
        assert_eq!(lines[2], " Every worktree below any of these folders");
        assert_eq!(lines[3], " streams into one feed (from the Astroshots app");
        // The row above is exactly full, so the separating space starts this one.
        assert_eq!(lines[4], "  preferences).");
        assert_eq!(lines[5], " ● ~/projects");
        // Long paths are cut at the end.
        assert_eq!(lines[6], " ● /work/a-very-long-directory-name-that-canno…");
        assert_eq!(fg_at(&buf, 1, 5), THEME.green);
        assert_eq!(lines[8], " GRAPHICS");
        assert_eq!(lines[9], " Images    Kitty graphics (pixel-perfect)");
        assert_eq!(lines[11], " Transport file path (local)");
        assert_eq!(lines[12], " Session   ssh · tmux");
        assert_eq!(lines[15], " ffmpeg    /opt/homebrew/bin/ffmpeg (7.1)");
        // Meta values are cut from the start when they do not fit.
        let narrow = rows(&draw(&roots, &services, 40, 24));
        assert_eq!(narrow[9], " Images    …ty graphics (pixel-perfect)");
    }

    #[tokio::test]
    async fn graphics_modes_describe_themselves_and_session_flags_join() {
        let mut services = services(GraphicsProtocol::Herdr, FakeService::new(false));
        services.capabilities.inside_herdr = true;
        services.capabilities.inside_mosh = true;
        let buf = draw(&[], &services, 60, 14);
        assert_eq!(
            rows(&buf)[7],
            " Images    herdr pane graphics (pixel-perfect)"
        );
        assert_eq!(rows(&buf)[10], " Session   local · mosh · herdr");

        services.capabilities.graphics = GraphicsProtocol::Halfblocks;
        services.capabilities.reason = Some("inside tmux".into());
        assert_eq!(
            rows(&draw(&[], &services, 60, 14))[7],
            " Images    half-block text · inside tmux"
        );
        services.capabilities.reason = None;
        assert_eq!(
            rows(&draw(&[], &services, 60, 14))[7],
            " Images    half-block text ·"
        );

        services.capabilities.graphics = GraphicsProtocol::None;
        assert_eq!(
            rows(&draw(&[], &services, 60, 14))[7],
            " Images    off · unsupported"
        );

        services.capabilities.cell_source = CellSource::Query;
        services.capabilities.cell_width = 9;
        services.capabilities.cell_height = 18;
        assert_eq!(
            rows(&draw(&[], &services, 60, 14))[8],
            " Cell      9×18 px (query)"
        );
    }

    #[tokio::test]
    async fn rows_past_the_height_are_clipped() {
        let services = services(GraphicsProtocol::None, FakeService::new(false));
        let buf = draw(&[], &services, 60, 6);
        assert_eq!(
            rows(&buf).last().map(String::as_str),
            Some(&*format!(" {}", "─".repeat(58)))
        );
        assert!(draw(&[], &services, 0, 6).area.width == 0);
    }

    #[tokio::test]
    async fn roots_source_labels_cover_cli_app_and_cwd() {
        let mut services = services(GraphicsProtocol::None, FakeService::new(false));
        let roots = vec!["/r".to_string()];
        for (source, label) in [
            (RootsSource::Cli, "from --root"),
            (RootsSource::App, "from the Astroshots app preferences"),
            (RootsSource::Cwd, "current directory"),
        ] {
            services.roots_source = source;
            let text = rows(&draw(&roots, &services, 120, 6)).join("\n");
            assert!(text.contains(&format!("one feed ({label}).")), "{text}");
        }
    }
}
