use crate::state::{
    unix_now, unix_now_ms, Activity, ClickRegion, FlashMode, MenuAction, MenuClickRegion,
    NotifyMode, SessionInfo, SettingKey, State, ViewMode,
};
use std::collections::HashMap;
use std::fmt::Write;
use std::io::Write as IoWrite;
use zellij_tile::prelude::{InputMode, TabInfo};

struct Style {
    symbol: &'static str,
    r: u8,
    g: u8,
    b: u8,
}

fn activity_style(activity: &Activity) -> Style {
    match activity {
        Activity::Init => Style {
            symbol: "◆",
            r: 126,
            g: 130,
            b: 148,
        },
        Activity::Thinking => Style {
            symbol: "●",
            r: 187,
            g: 151,
            b: 238,
        },
        Activity::Tool(name) => {
            let symbol = match name.as_str() {
                "Bash" => "⚡",
                "Read" | "Glob" | "Grep" => "◉",
                "Edit" | "Write" => "✎",
                "Task" => "⊜",
                "WebSearch" | "WebFetch" => "◈",
                _ => "⚙",
            };
            Style {
                symbol,
                r: 248,
                g: 152,
                b: 96,
            }
        }
        Activity::Prompting => Style {
            symbol: "▶",
            r: 158,
            g: 208,
            b: 108,
        },
        Activity::Waiting => Style {
            symbol: "⚠",
            r: 251,
            g: 97,
            b: 126,
        },
        Activity::Notification => Style {
            symbol: "◇",
            r: 237,
            g: 199,
            b: 99,
        },
        Activity::Done => Style {
            symbol: "✓",
            r: 158,
            g: 208,
            b: 108,
        },
        Activity::AgentDone => Style {
            symbol: "✓",
            r: 130,
            g: 190,
            b: 90,
        },
        Activity::Idle => Style {
            symbol: "○",
            r: 126,
            g: 130,
            b: 148,
        },
    }
}

fn fg(r: u8, g: u8, b: u8) -> String {
    format!("\x1b[38;2;{r};{g};{b}m")
}

fn bg(r: u8, g: u8, b: u8) -> String {
    format!("\x1b[48;2;{r};{g};{b}m")
}

fn display_width(s: &str) -> usize {
    s.chars().count()
}

const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
const ELAPSED_THRESHOLD: u64 = 30;
const SEPARATOR: &str = "\u{e0b0}";

type Color = (u8, u8, u8);
const BAR_BG: Color = (43, 45, 58);
const PREFIX_BG: Color = (43, 45, 58);
const PREFIX_BG_SETTINGS: Color = (63, 68, 91);
const TAB_BG_ACTIVE: Color = (63, 68, 91);
const TAB_BG_INACTIVE: Color = (33, 35, 46);
const FLASH_BG_BRIGHT: Color = (80, 70, 25);

/// Write a powerline arrow: fg=from_bg, bg=to_bg, then separator char.
fn arrow(buf: &mut String, col: &mut usize, from: Color, to: Color) {
    let _ = write!(
        buf,
        "{}{}{SEPARATOR}",
        fg(from.0, from.1, from.2),
        bg(to.0, to.1, to.2),
    );
    *col += 1;
}

fn format_elapsed(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else {
        format!("{}h", secs / 3600)
    }
}

fn mode_style(mode: InputMode) -> (Color, &'static str) {
    match mode {
        InputMode::Normal => ((158, 208, 108), "NORMAL"),
        InputMode::Locked => ((251, 97, 126), "LOCKED"),
        InputMode::Pane => ((109, 202, 232), "PANE"),
        InputMode::Tab => ((187, 151, 238), "TAB"),
        InputMode::Resize => ((248, 152, 96), "RESIZE"),
        InputMode::Move => ((248, 152, 96), "MOVE"),
        InputMode::Scroll => ((237, 199, 99), "SCROLL"),
        InputMode::EnterSearch => ((237, 199, 99), "SEARCH"),
        InputMode::Search => ((237, 199, 99), "SEARCH"),
        InputMode::RenameTab => ((237, 199, 99), "RENAME"),
        InputMode::RenamePane => ((237, 199, 99), "RENAME"),
        InputMode::Session => ((187, 151, 238), "SESSION"),
        InputMode::Prompt => ((158, 208, 108), "PROMPT"),
        InputMode::Tmux => ((158, 208, 108), "TMUX"),
    }
}

pub fn render_status_bar(state: &mut State, _rows: usize, cols: usize) {
    state.click_regions.clear();
    state.menu_click_regions.clear();

    let mut buf = String::with_capacity(cols * 4);
    // Terminal setup for a 1-row status bar:
    //  \x1b[H     — cursor home (prevent scroll from cursor at end-of-line)
    //  \x1b[?7l   — disable auto-wrap (clip overflow instead of scroll)
    //  \x1b[?25l  — hide cursor
    buf.push_str("\x1b[H\x1b[?7l\x1b[?25l");
    let bar_bg_str = bg(BAR_BG.0, BAR_BG.1, BAR_BG.2);

    // Bail early if terminal is too narrow
    if cols < 5 {
        let _ = write!(buf, "{bar_bg_str}{:width$}{RESET}", "", width = cols);
        print!("{buf}");
        let _ = std::io::stdout().flush();
        return;
    }

    let prefix_bg = if state.view_mode == ViewMode::Settings {
        PREFIX_BG_SETTINGS
    } else {
        PREFIX_BG
    };

    // Build prefix: " Zellaude (session) MODE "
    let (mode_bg, mode_text) = mode_style(state.input_mode);
    let show_mode = state.settings.mode_indicator;
    let session_part = match state.zellij_session_name.as_deref() {
        Some(name) => format!(" ({name})"),
        None => String::new(),
    };
    let prefix_text = format!(" Zellaude{session_part} ");
    let prefix_width = display_width(&prefix_text);
    let mode_pill_width = if show_mode {
        1 + mode_text.len() + 1
    } else {
        0
    };
    let total_prefix_width = prefix_width + mode_pill_width;

    // Render prefix segment (truncate if wider than cols)
    let mut col;
    if total_prefix_width <= cols {
        let _ = write!(
            buf,
            "{}{}{BOLD}{prefix_text}{RESET}",
            bg(prefix_bg.0, prefix_bg.1, prefix_bg.2),
            fg(255, 255, 255),
        );
        if show_mode {
            let _ = write!(
                buf,
                "{}{}{BOLD} {mode_text} {RESET}",
                bg(mode_bg.0, mode_bg.1, mode_bg.2),
                fg(24, 26, 28),
            );
        }
        col = total_prefix_width;
    } else if prefix_width <= cols {
        // Fit the name part but skip mode pill
        let _ = write!(
            buf,
            "{}{}{BOLD}{prefix_text}{RESET}",
            bg(prefix_bg.0, prefix_bg.1, prefix_bg.2),
            fg(255, 255, 255),
        );
        col = prefix_width;
    } else {
        // Even name doesn't fit — just show what we can
        let avail = cols.saturating_sub(2); // leave room for fill
        let short: String = prefix_text.chars().take(avail).collect();
        let _ = write!(
            buf,
            "{}{}{BOLD}{short}{RESET}",
            bg(prefix_bg.0, prefix_bg.1, prefix_bg.2),
            fg(255, 255, 255),
        );
        col = display_width(&short);
    }
    state.prefix_click_region = Some((0, col));

    let last_prefix_bg = if show_mode && total_prefix_width <= cols {
        mode_bg
    } else {
        prefix_bg
    };
    let prefix_used = col;

    if col < cols {
        match state.view_mode {
            ViewMode::Normal => {
                render_tabs(state, &mut buf, &mut col, cols, last_prefix_bg, prefix_used);
            }
            ViewMode::Settings => {
                arrow(&mut buf, &mut col, last_prefix_bg, BAR_BG);
                let _ = write!(buf, "{bar_bg_str}");
                render_settings_menu(state, &mut buf, &mut col);
            }
        }
    }

    // Fill remaining width with bar background — never exceed cols
    if col < cols {
        let remaining = cols - col;
        let _ = write!(buf, "{bar_bg_str}{:width$}", "", width = remaining);
    }
    let _ = write!(buf, "{RESET}");

    print!("{buf}");
    let _ = std::io::stdout().flush();
}

fn render_tabs(
    state: &mut State,
    buf: &mut String,
    col: &mut usize,
    cols: usize,
    prefix_bg: Color,
    prefix_width: usize,
) {
    let now_s = unix_now();
    let now_ms = unix_now_ms();

    // Sort tabs by position
    let mut tabs: Vec<&TabInfo> = state.tabs.iter().collect();
    tabs.sort_by_key(|t| t.position);

    let count = tabs.len();
    if count == 0 {
        arrow(buf, col, prefix_bg, BAR_BG);
        return;
    }

    // Per-tab pane geometry — used to order session markers row-major
    // (top-to-bottom, left-to-right) within each tab. Plugin panes are
    // excluded because PaneInfo.id is only unique within kind (terminal
    // vs plugin), so a plugin pane can collide with a terminal pane on
    // the same numeric id and stomp the terminal's coords with its own.
    let pane_positions: HashMap<usize, HashMap<u32, (usize, usize)>> = state
        .pane_manifest
        .as_ref()
        .map(|m| {
            m.panes
                .iter()
                .map(|(&tab_idx, panes)| {
                    let positions: HashMap<u32, (usize, usize)> = panes
                        .iter()
                        .filter(|p| !p.is_plugin)
                        .map(|p| (p.id, (p.pane_y, p.pane_x)))
                        .collect();
                    (tab_idx, positions)
                })
                .collect()
        })
        .unwrap_or_default();

    let sessions_per_tab: Vec<Vec<&SessionInfo>> = tabs
        .iter()
        .map(|tab| {
            let mut v: Vec<&SessionInfo> = state
                .sessions
                .values()
                .filter(|s| s.tab_index == Some(tab.position))
                .collect();
            if let Some(positions) = pane_positions.get(&tab.position) {
                v.sort_by_key(|s| {
                    positions
                        .get(&s.pane_id)
                        .copied()
                        .unwrap_or((usize::MAX, usize::MAX))
                });
            }
            v
        })
        .collect();

    // Elapsed indicator: only meaningful for tabs with a single session.
    // Multi-session tabs suppress it to keep the marker row readable.
    let elapsed_strs: Vec<Option<String>> = sessions_per_tab
        .iter()
        .map(|sessions| {
            if !state.settings.elapsed_time || sessions.len() != 1 {
                return None;
            }
            let s = sessions[0];
            let elapsed = now_s.saturating_sub(s.last_event_ts);
            if elapsed >= ELAPSED_THRESHOLD {
                Some(format_elapsed(elapsed))
            } else {
                None
            }
        })
        .collect();

    // Per-tab overhead: 2 (leading + trailing space) + 2*N (each session contributes
    // a symbol cell + a separator space). Empty (non-agent) tabs cost 2.
    let total_elapsed_width: usize = elapsed_strs
        .iter()
        .map(|e: &Option<String>| e.as_ref().map_or(0, |s| s.len() + 1))
        .sum();
    let per_tab_overhead: usize = sessions_per_tab
        .iter()
        .map(
            |sl: &Vec<&SessionInfo>| {
                if sl.is_empty() {
                    2
                } else {
                    2 + 2 * sl.len()
                }
            },
        )
        .sum();
    let overhead = prefix_width + 2 * count + per_tab_overhead + total_elapsed_width;
    let max_name_len = if overhead < cols {
        ((cols - overhead) / count).min(20)
    } else {
        0
    };

    let mut prev_bg = prefix_bg;

    for (i, tab) in tabs.iter().enumerate() {
        let sessions = &sessions_per_tab[i];
        let tab_overhead = if sessions.is_empty() {
            2
        } else {
            2 + 2 * sessions.len()
        };
        // Don't start a tab we can't render in full: arrows + leading space + at least
        // every marker + trailing space. Reserves at least 3 cols for non-agent tabs.
        let arrows_needed = if prev_bg == prefix_bg { 1 } else { 2 };
        if *col + arrows_needed + tab_overhead.max(3) > cols {
            break;
        }

        let is_agent = !sessions.is_empty();
        let tab_name = &tab.name;
        let char_count = tab_name.chars().count();
        let truncated = if max_name_len == 0 {
            String::new()
        } else if char_count > max_name_len {
            let s: String = tab_name
                .chars()
                .take(max_name_len.saturating_sub(1))
                .collect();
            format!("{s}…")
        } else {
            tab_name.to_string()
        };

        // Flash if any session in this tab is currently flashing.
        let is_flash_bright = sessions.iter().any(|s| {
            state
                .flash_deadlines
                .get(&s.pane_id)
                .map(|&deadline| now_ms < deadline && (now_ms / 250).is_multiple_of(2))
                .unwrap_or(false)
        });

        let is_active = tab.active;
        let tab_bg = if is_flash_bright {
            FLASH_BG_BRIGHT
        } else if is_active {
            TAB_BG_ACTIVE
        } else {
            TAB_BG_INACTIVE
        };

        if prev_bg == prefix_bg {
            arrow(buf, col, prev_bg, tab_bg);
        } else {
            arrow(buf, col, prev_bg, BAR_BG);
            arrow(buf, col, BAR_BG, tab_bg);
        }

        let tab_bg_str = bg(tab_bg.0, tab_bg.1, tab_bg.2);
        let region_start = *col;

        if is_agent {
            let (name_fg, name_bold) = if is_flash_bright {
                (fg(237, 199, 99), true)
            } else if is_active {
                (fg(225, 227, 228), true)
            } else {
                (fg(126, 130, 148), false)
            };

            // Leading space
            let _ = write!(buf, "{tab_bg_str} ");
            *col += 1;

            // One marker per session, in pane-geometry order. Waiting markers get
            // their own click region so a click on a specific marker focuses that
            // pane (not just any waiting pane in the tab).
            for s in sessions {
                let style = activity_style(&s.activity);
                let sym_fg = if is_flash_bright {
                    fg(237, 199, 99)
                } else {
                    fg(style.r, style.g, style.b)
                };
                let marker_start = *col;
                let _ = write!(buf, "{sym_fg}{}", style.symbol);
                *col += display_width(style.symbol);
                let marker_end = *col;
                let _ = write!(buf, "{tab_bg_str} ");
                *col += 1;

                if matches!(s.activity, Activity::Waiting) {
                    state.click_regions.push(ClickRegion {
                        start_col: marker_start,
                        end_col: marker_end,
                        tab_index: tab.position,
                        pane_id: s.pane_id,
                        is_waiting: true,
                    });
                }
            }

            // Tab name (already preceded by the trailing space of the last marker)
            if !truncated.is_empty() {
                let bold_str = if name_bold { BOLD } else { "" };
                let _ = write!(buf, "{bold_str}{name_fg}{truncated}{RESET}{tab_bg_str}");
                *col += display_width(&truncated);
            }

            if let Some(ref es) = elapsed_strs[i] {
                if *col + 1 + es.len() + 1 < cols {
                    let _ = write!(buf, " {}{es}", fg(126, 130, 148));
                    *col += 1 + es.len();
                }
            }

            if tab.is_fullscreen_active && *col + 3 < cols {
                let _ = write!(buf, " {}F{RESET}{tab_bg_str}", fg(237, 199, 99));
                *col += 2;
            }

            // Trailing space
            let _ = write!(buf, " ");
            *col += 1;

            // Catch-all click region for the tab segment. Pushed after per-marker
            // waiting regions so the click handler matches a waiting marker first.
            state.click_regions.push(ClickRegion {
                start_col: region_start,
                end_col: *col,
                tab_index: tab.position,
                pane_id: 0,
                is_waiting: false,
            });
        } else {
            let name_fg = if is_active {
                fg(225, 227, 228)
            } else {
                fg(126, 130, 148)
            };
            let name_bold = is_active;

            let _ = write!(buf, "{tab_bg_str} ");
            *col += 1;

            if !truncated.is_empty() {
                let bold_str = if name_bold { BOLD } else { "" };
                let _ = write!(buf, "{bold_str}{name_fg}{truncated}{RESET}{tab_bg_str}");
                *col += display_width(&truncated);
            }

            if tab.is_fullscreen_active && *col + 3 < cols {
                let _ = write!(buf, " {}F{RESET}{tab_bg_str}", fg(237, 199, 99));
                *col += 2;
            }

            let _ = write!(buf, " ");
            *col += 1;

            state.click_regions.push(ClickRegion {
                start_col: region_start,
                end_col: *col,
                tab_index: tab.position,
                pane_id: 0,
                is_waiting: false,
            });
        }

        prev_bg = tab_bg;
    }

    if prev_bg != prefix_bg || count > 0 {
        arrow(buf, col, prev_bg, BAR_BG);
    }
}

fn notify_mode_label(mode: NotifyMode) -> (&'static str, &'static str, String, String) {
    match mode {
        NotifyMode::Always => ("●", "Notify: always", fg(158, 208, 108), fg(225, 227, 228)),
        NotifyMode::Unfocused => ("◐", "Notify: unfocused", fg(237, 199, 99), fg(237, 199, 99)),
        NotifyMode::Never => ("○", "Notify: off", fg(126, 130, 148), fg(126, 130, 148)),
    }
}

fn flash_mode_label(mode: FlashMode) -> (&'static str, &'static str, String, String) {
    match mode {
        FlashMode::Persist => ("●", "Flash: persist", fg(158, 208, 108), fg(225, 227, 228)),
        FlashMode::Once => ("◐", "Flash: brief", fg(237, 199, 99), fg(237, 199, 99)),
        FlashMode::Off => ("○", "Flash: off", fg(126, 130, 148), fg(126, 130, 148)),
    }
}

/// Render a three-state toggle and register its click region.
/// Assumes the caller has already set the desired background color.
fn render_tristate(
    buf: &mut String,
    col: &mut usize,
    state_regions: &mut Vec<MenuClickRegion>,
    key: SettingKey,
    symbol: &str,
    label: &str,
    colors: (&str, &str),
) {
    let region_start = *col;
    let width = display_width(symbol) + 1 + label.len();
    *col += width;

    state_regions.push(MenuClickRegion {
        start_col: region_start,
        end_col: *col,
        action: MenuAction::ToggleSetting(key),
    });

    let _ = write!(buf, "{}{}  {}{}", colors.0, symbol, colors.1, label);
}

fn render_settings_menu(state: &mut State, buf: &mut String, col: &mut usize) {
    // Leading space after arrow
    let _ = write!(buf, " ");
    *col += 1;

    // --- Notifications (three-state) ---
    {
        let (symbol, label, sym_color, label_color) =
            notify_mode_label(state.settings.notifications);
        render_tristate(
            buf,
            col,
            &mut state.menu_click_regions,
            SettingKey::Notifications,
            symbol,
            label,
            (&sym_color, &label_color),
        );
    }

    // --- Flash (three-state) ---
    {
        let _ = write!(buf, "  ");
        *col += 2;
        let (symbol, label, sym_color, label_color) = flash_mode_label(state.settings.flash);
        render_tristate(
            buf,
            col,
            &mut state.menu_click_regions,
            SettingKey::Flash,
            symbol,
            label,
            (&sym_color, &label_color),
        );
    }

    // --- Elapsed time (bool) ---
    {
        let _ = write!(buf, "  ");
        *col += 2;
        let enabled = state.settings.elapsed_time;
        let (symbol, sym_color, label_color) = if enabled {
            ("●", fg(158, 208, 108), fg(225, 227, 228))
        } else {
            ("○", fg(126, 130, 148), fg(126, 130, 148))
        };
        let label = if enabled {
            "Elapsed time: on"
        } else {
            "Elapsed time: off"
        };
        render_tristate(
            buf,
            col,
            &mut state.menu_click_regions,
            SettingKey::ElapsedTime,
            symbol,
            label,
            (&sym_color, &label_color),
        );
    }

    // --- Mode indicator (bool) ---
    {
        let _ = write!(buf, "  ");
        *col += 2;
        let enabled = state.settings.mode_indicator;
        let (symbol, sym_color, label_color) = if enabled {
            ("●", fg(80, 200, 120), fg(255, 255, 255))
        } else {
            ("○", fg(100, 100, 100), fg(100, 100, 100))
        };
        let label = if enabled {
            "Mode indicator: on"
        } else {
            "Mode indicator: off"
        };
        render_tristate(
            buf,
            col,
            &mut state.menu_click_regions,
            SettingKey::ModeIndicator,
            symbol,
            label,
            (&sym_color, &label_color),
        );
    }

    // Close button
    let _ = write!(buf, "  ");
    *col += 2;
    let close_start = *col;
    let _ = write!(buf, "{}×", fg(251, 97, 126));
    *col += 1;

    state.menu_click_regions.push(MenuClickRegion {
        start_col: close_start,
        end_col: *col,
        action: MenuAction::CloseMenu,
    });
}
