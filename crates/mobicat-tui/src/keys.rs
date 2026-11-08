//! Key handling for every mode.

use std::io::Write as _;
use std::sync::Arc;
use std::time::Instant;

use mobicat_core::output::write_plain;
use mobicat_core::{Entry, Filter, Level, Query, Source, Store, View};
use ratatui::crossterm::event::{KeyCode, KeyModifiers};

use crate::palette::{self, Act, Palette, PaletteResult};
use crate::picker::{Picker, PickerResult};
use crate::targets::{self, PanelResult, TargetPanel};
use crate::{App, InputKind, InputState, Mode, QUIT_WINDOW, Tone, clipboard, complete};

pub(crate) fn handle_key(app: &mut App, code: KeyCode, mods: KeyModifiers) {
    let ctrl = mods.contains(KeyModifiers::CONTROL);
    if ctrl && matches!(code, KeyCode::Char('c') | KeyCode::Char('d')) {
        let key = if code == KeyCode::Char('c') {
            "Ctrl-C"
        } else {
            "Ctrl-D"
        };
        arm_quit(app, key);
        return;
    }
    if app.help {
        app.help = false;
        return;
    }
    match std::mem::replace(&mut app.mode, Mode::Normal) {
        Mode::Normal => normal(app, code, mods),
        Mode::Input(st) => input(app, st, code, ctrl),
        Mode::Picker(mut p) => match p.key(code) {
            PickerResult::Open => app.mode = Mode::Picker(p),
            PickerResult::Close => {}
            PickerResult::Add(term) => add_term(app, &term),
        },
        Mode::Targets(mut p) => {
            let store = app.store.clone();
            let r = p.key(&store.lock().expect("store lock"), code, ctrl);
            match r {
                PanelResult::Open => app.mode = Mode::Targets(p),
                PanelResult::Close => {}
                PanelResult::Switch(v) => app.switch_to(v),
                PanelResult::ChangeApp => {
                    app.stats.change_app = true;
                    app.quit = true;
                }
            }
        }
        Mode::Palette(mut p) => match p.key(code, ctrl) {
            PaletteResult::Open => app.mode = Mode::Palette(p),
            PaletteResult::Close => {}
            PaletteResult::Run(i) => run_action(app, i),
        },
        Mode::ConfirmClear => {
            if matches!(code, KeyCode::Char('y') | KeyCode::Char('Y')) {
                app.lock().clear();
                app.viewports.values_mut().for_each(|p| p.reset());
                app.detail = false;
                app.visual = None;
                app.toast_as(Tone::Ok, "✓ cleared");
            }
        }
    }
}

/// First press shows "Press … again to exit"; a second one within the window quits.
fn arm_quit(app: &mut App, key: &'static str) {
    match app.quit_armed {
        Some((t, _)) if t.elapsed() <= QUIT_WINDOW => app.quit = true,
        _ => app.quit_armed = Some((Instant::now(), key)),
    }
}

/// Esc / q: close the nearest open thing. Returns false when nothing was open.
fn back(app: &mut App) -> bool {
    if app.visual.is_some() {
        end_visual(app);
    } else if app.detail {
        app.detail = false;
    } else if app.vp().cursor().is_some() {
        app.vp().follow();
    } else {
        return false;
    }
    true
}

fn run_action(app: &mut App, i: usize) {
    match palette::ACTIONS[i].2 {
        Act::Quit => app.quit = true,
        Act::ChangeApp => {
            app.stats.change_app = true;
            app.quit = true;
        }
        Act::Key(code, mods) => {
            if let Some(first) = palette::chord(i) {
                app.pending = Some(first);
            }
            handle_key(app, code, mods);
        }
    }
}

fn input(app: &mut App, mut st: InputState, code: KeyCode, ctrl: bool) {
    let mut edited = true;
    match code {
        KeyCode::Esc => {
            if st.kind == InputKind::Query {
                app.query_text = st.before_text;
                set_filter(app, st.before);
            }
            return;
        }
        KeyCode::Enter => {
            match st.kind {
                InputKind::Query => {
                    if st.error.is_some() {
                        // Keep editing until it parses.
                        app.mode = Mode::Input(st);
                        return;
                    }
                }
                InputKind::Search => match Query::parse(&st.text) {
                    Ok(q) if !q.is_empty() => {
                        app.search = Some(Arc::new(q));
                        search(app, false);
                    }
                    Ok(_) => app.search = None,
                    Err(e) => {
                        st.error = Some(e);
                        app.mode = Mode::Input(st);
                        return;
                    }
                },
            }
            app.remember(&st.text);
            return;
        }
        KeyCode::Tab | KeyCode::BackTab => {
            let (start, items) = st.suggest.clone();
            if items.is_empty() {
                edited = false;
            } else {
                let (base, i) = match st.cycling.take() {
                    None => (st.text.clone(), 0),
                    Some((base, i)) => {
                        let n = items.len();
                        let next = if code == KeyCode::Tab {
                            (i + 1) % n
                        } else {
                            (i + n - 1) % n
                        };
                        (base, next)
                    }
                };
                st.text = format!("{}{}", &base[..start.min(base.len())], items[i]);
                if items.len() == 1 {
                    // Only one way to go: take it and offer what comes next.
                    st.cycling = None;
                } else {
                    st.cycling = Some((base, i));
                }
            }
        }
        KeyCode::Up | KeyCode::Down => {
            edited = false;
            if !app.history.is_empty() {
                let last = app.history.len() - 1;
                let next = match (st.hist.take(), code) {
                    (None, KeyCode::Up) => Some((last, st.text.clone())),
                    (None, _) => None,
                    (Some((i, draft)), KeyCode::Up) => Some((i.saturating_sub(1), draft)),
                    (Some((i, draft)), _) if i < last => Some((i + 1, draft)),
                    (Some((_, draft)), _) => {
                        st.text = draft;
                        None
                    }
                };
                if let Some((i, draft)) = next {
                    st.text = app.history[i].clone();
                    st.hist = Some((i, draft));
                }
                edited = true;
            }
        }
        KeyCode::Backspace => {
            st.text.pop();
        }
        KeyCode::Char('u') if ctrl => st.text.clear(),
        KeyCode::Char('w') if ctrl => {
            let keep = st.text.trim_end().rfind(' ').map_or(0, |i| i + 1);
            st.text.truncate(keep);
        }
        KeyCode::Char(c) => st.text.push(c),
        _ => edited = false,
    }
    if edited {
        if !matches!(code, KeyCode::Tab | KeyCode::BackTab) {
            st.cycling = None;
        }
        if !matches!(code, KeyCode::Up | KeyCode::Down) {
            st.hist = None;
        }
        match Query::parse(&st.text) {
            Ok(q) => {
                st.error = None;
                if st.kind == InputKind::Query {
                    // Live: apply as you type.
                    let mut f = app.lock().filter().clone();
                    f.query = (!q.is_empty()).then(|| Arc::new(q));
                    app.query_text = st.text.clone();
                    set_filter(app, f);
                }
            }
            Err(e) => st.error = Some(e),
        }
        // Keep the candidates of the first Tab while cycling through them.
        if st.cycling.is_none() {
            st.suggest = complete::suggest(&st.text, &app.lock(), app.target);
        }
    }
    app.mode = Mode::Input(st);
}

fn normal(app: &mut App, code: KeyCode, mods: KeyModifiers) {
    let ctrl = mods.contains(KeyModifiers::CONTROL);
    if let Some(first) = app.pending.take() {
        let fwd = first == ']';
        match code {
            KeyCode::Char('c') => jump(app, fwd, |e| e.crash, "crash"),
            KeyCode::Char('e') => jump(app, fwd, is_error, "error"),
            KeyCode::Char('w') => jump(app, fwd, |e| e.level == Level::Warn, "warning"),
            _ => {}
        }
        return;
    }
    let store_arc = app.store.clone();
    let store = store_arc.lock().expect("store lock");
    match code {
        KeyCode::Char('q') | KeyCode::Esc => {
            drop(store);
            if !back(app) && code == KeyCode::Char('q') {
                arm_quit(app, "q");
            }
            return;
        }
        KeyCode::Char('?') => app.help = true,
        KeyCode::Char(']') | KeyCode::Char('[') => {
            if let KeyCode::Char(c) = code {
                app.pending = Some(c);
            }
        }
        KeyCode::Char('j') | KeyCode::Down => app.vp().down(&store, 1),
        KeyCode::Char('k') | KeyCode::Up => app.vp().up(&store, 1),
        KeyCode::PageDown => app.vp().page_down(&store),
        KeyCode::PageUp => app.vp().page_up(&store),
        KeyCode::Char('g') | KeyCode::Home => app.vp().first(&store),
        KeyCode::Char('G') | KeyCode::End => app.vp().follow(),
        KeyCode::Char(' ') => app.vp().toggle_pause(&store),
        KeyCode::Char('o') => app.vp().toggle_expand(&store),
        KeyCode::Enter => {
            app.detail = !app.detail;
            if app.detail && app.vp().cursor().is_none() {
                app.vp().toggle_pause(&store);
            }
        }
        KeyCode::Char('w') if ctrl => app.layout.wrap = !app.layout.wrap,
        KeyCode::Char('s') if ctrl => {
            let (ok, msg) = save(&store, app.target, &app.clock);
            drop(store);
            app.toast_as(if ok { Tone::Ok } else { Tone::Warn }, msg);
            return;
        }
        KeyCode::Char('T') => app.layout.time = app.layout.time.next(),
        KeyCode::Char('V') => {
            if app.visual.is_some() {
                end_visual(app);
            } else {
                if app.vp().cursor().is_none() {
                    app.vp().toggle_pause(&store);
                }
                app.visual = app.vp().cursor();
            }
        }
        KeyCode::Char('y') => {
            let view = app.target;
            let text = match (app.visual, app.vp().cursor()) {
                (Some(a), Some(c)) => range_text(&store, view, a.min(c), a.max(c), &app.clock),
                (None, Some(c)) => store
                    .get(c)
                    .map(|e| entry_text(e, &app.clock))
                    .unwrap_or_default(),
                (_, None) => store
                    .last_filtered(view)
                    .and_then(|s| store.get(s))
                    .map(|e| entry_text(e, &app.clock))
                    .unwrap_or_default(),
            };
            drop(store);
            end_visual(app);
            copy(app, &text);
            return;
        }
        KeyCode::Char('Y') => {
            let view = app.target;
            let text = match (store.filtered_seq(view, 0), store.last_filtered(view)) {
                (Some(a), Some(b)) => range_text(&store, view, a, b, &app.clock),
                _ => String::new(),
            };
            drop(store);
            copy(app, &text);
            return;
        }
        KeyCode::Char('n') | KeyCode::Char('N') => {
            drop(store);
            search(app, code == KeyCode::Char('n'));
            return;
        }
        KeyCode::Char('e') | KeyCode::Char('E') => {
            drop(store);
            jump(app, code == KeyCode::Char('e'), is_error, "error");
            return;
        }
        KeyCode::Char('w') | KeyCode::Char('W') => {
            drop(store);
            jump(
                app,
                code == KeyCode::Char('w'),
                |e| e.level == Level::Warn,
                "warning",
            );
            return;
        }
        KeyCode::Char('/') | KeyCode::Char('f') => {
            let kind = if code == KeyCode::Char('/') {
                InputKind::Query
            } else {
                InputKind::Search
            };
            let text = match kind {
                InputKind::Query => app.query_text.clone(),
                InputKind::Search => app
                    .search
                    .as_ref()
                    .map(|q| q.as_str().to_owned())
                    .unwrap_or_default(),
            };
            let suggest = complete::suggest(&text, &store, app.target);
            app.mode = Mode::Input(InputState {
                kind,
                text,
                error: None,
                before: store.filter().clone(),
                before_text: app.query_text.clone(),
                suggest,
                cycling: None,
                hist: None,
            });
        }
        KeyCode::Char('t') => {
            let picker = Picker::from_store(&store, app.target);
            app.mode = Mode::Picker(picker);
        }
        KeyCode::Char('=') | KeyCode::Char('!') => {
            let tag = app
                .vp()
                .cursor()
                .and_then(|s| store.get(s))
                .filter(|e| e.source != Source::System)
                .map(|e| e.tag.to_string());
            drop(store);
            if let Some(tag) = tag {
                let neg = if code == KeyCode::Char('!') { "-" } else { "" };
                add_term(app, &format!("{neg}tag:{}", quote(&tag)));
            }
            return;
        }
        KeyCode::Char(c @ '1'..='9') => {
            drop(store);
            let i = c as usize - '1' as usize;
            match app.profiles.get(i).cloned() {
                Some((name, q)) => {
                    if set_query(app, &q) {
                        app.say(format!("profile {name}"));
                    }
                }
                None => app.say(format!(
                    "no profile {c} (add [profiles.NAME] to .mobicat.toml)"
                )),
            }
            return;
        }
        KeyCode::Tab | KeyCode::BackTab => {
            let next = targets::cycle(&store, app.target, code == KeyCode::Tab);
            drop(store);
            app.switch_to(next);
            return;
        }
        KeyCode::Char('d') => {
            app.mode = Mode::Targets(TargetPanel::new(&store, app.target, app.can_change_app));
        }
        KeyCode::Char(':') => app.mode = Mode::Palette(Palette::new(app.can_change_app)),
        KeyCode::Char('x') => {
            drop(store);
            if app.query_text.is_empty() {
                app.say("x switches how the query works; set one with / first");
            } else {
                change_filter(app, |f| f.highlight = !f.highlight);
            }
            return;
        }
        KeyCode::Char('+') => {
            drop(store);
            change_filter(app, |f| f.min_level = f.min_level.up());
            return;
        }
        KeyCode::Char('-') => {
            drop(store);
            change_filter(app, |f| f.min_level = f.min_level.down());
            return;
        }
        KeyCode::Char('a') => {
            drop(store);
            change_filter(app, |f| f.app_only = !f.app_only);
            return;
        }
        KeyCode::Char('c') => app.mode = Mode::ConfirmClear,
        _ => {}
    }
    drop(store);
    if let Some(anchor) = app.visual {
        let cur = app.vp().cursor().unwrap_or(anchor);
        app.vp().marked = Some((anchor.min(cur), anchor.max(cur)));
    }
}

fn is_error(e: &Entry) -> bool {
    e.level >= Level::Error && e.source != Source::System
}

fn end_visual(app: &mut App) {
    app.visual = None;
    app.viewports.values_mut().for_each(|p| p.marked = None);
}

fn quote(v: &str) -> String {
    if v.is_empty() || v.chars().any(|c| c.is_whitespace() || "()|&\"".contains(c)) {
        format!("\"{}\"", v.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        v.to_owned()
    }
}

/// Append a term to the query (AND with what is there).
fn add_term(app: &mut App, term: &str) {
    let q = if app.query_text.trim().is_empty() {
        term.to_owned()
    } else {
        format!("{} {term}", app.query_text.trim_end())
    };
    if set_query(app, &q) {
        app.say(format!("filter: {q}"));
    }
}

/// Parse and apply a query. Reports a parse error and returns false.
pub(crate) fn set_query(app: &mut App, text: &str) -> bool {
    match Query::parse(text) {
        Ok(q) => {
            let mut f = app.lock().filter().clone();
            f.query = (!q.is_empty()).then(|| Arc::new(q));
            app.query_text = text.to_owned();
            set_filter(app, f);
            true
        }
        Err(e) => {
            app.say(format!("query error: {e}"));
            false
        }
    }
}

fn set_filter(app: &mut App, filter: Filter) {
    let mut store = app.lock();
    let t0 = Instant::now();
    store.set_filter(filter);
    let took = t0.elapsed();
    let n = store.len();
    drop(store);
    app.stats.record_refilter(took, n);
}

fn change_filter(app: &mut App, f: impl FnOnce(&mut Filter)) {
    let mut filter = app.lock().filter().clone();
    f(&mut filter);
    set_filter(app, filter);
}

/// Move the focused pane's cursor to the nearest entry satisfying `pred`.
fn jump(app: &mut App, forward: bool, pred: impl Fn(&Entry) -> bool, what: &str) {
    let store_arc = app.store.clone();
    let store = store_arc.lock().expect("store lock");
    let view = app.target;
    let from = app.vp().position(&store);
    match store.find(view, from, forward, |e| pred(e)) {
        Some(pos) => app.vp().jump_to(&store, pos),
        None => {
            drop(store);
            let dir = if forward { "newer" } else { "older" };
            app.say(format!("no {dir} {what}"));
        }
    }
}

/// `forward` = newer. A new search starts upward from the cursor (or the newest line).
fn search(app: &mut App, forward: bool) {
    let Some(q) = app.search.clone() else {
        app.say("no search; start one with f");
        return;
    };
    let now = mobicat_core::store::now_ns();
    jump(app, forward, move |e| q.matches(e, now), "match");
}

fn entry_text(e: &Entry, clock: &mobicat_core::Clock) -> String {
    let mut out = Vec::new();
    let _ = write_plain(&mut out, e, clock, false);
    String::from_utf8_lossy(&out).into_owned()
}

fn range_text(
    store: &Store,
    view: View,
    from: mobicat_core::Seq,
    to: mobicat_core::Seq,
    clock: &mobicat_core::Clock,
) -> String {
    let start = store.filtered_position(view, from);
    let end = store.filtered_position(view, to.saturating_add(1));
    let mut out = Vec::new();
    for i in start..end {
        if let Some(e) = store.filtered_seq(view, i).and_then(|s| store.get(s)) {
            let _ = write_plain(&mut out, e, clock, false);
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn copy(app: &mut App, text: &str) {
    if text.is_empty() {
        app.say("nothing to copy");
        return;
    }
    let lines = text.lines().count();
    match clipboard::copy(text) {
        Ok(how) => app.toast_as(
            Tone::Ok,
            format!(
                "✓ copied {lines} line{} ({how})",
                if lines == 1 { "" } else { "s" }
            ),
        ),
        Err(e) => app.toast_as(Tone::Warn, format!("copy failed: {e}")),
    }
}

/// Save the view to `mobicat-YYYYMMDD-HHMMSS.log` in the current directory.
fn save(store: &Store, view: View, clock: &mobicat_core::Clock) -> (bool, String) {
    let name = jiff::Zoned::now()
        .strftime("mobicat-%Y%m%d-%H%M%S.log")
        .to_string();
    let text = match (store.filtered_seq(view, 0), store.last_filtered(view)) {
        (Some(a), Some(b)) => range_text(store, view, a, b, clock),
        _ => return (false, "nothing to save".into()),
    };
    let result = std::fs::File::create(&name).and_then(|mut f| f.write_all(text.as_bytes()));
    match result {
        Ok(()) => (
            true,
            format!("✓ saved {} lines to {name}", text.lines().count()),
        ),
        Err(e) => (false, format!("could not save {name}: {e}")),
    }
}

#[cfg(test)]
pub(crate) fn range_text_for_test(store: &Store, clock: &mobicat_core::Clock) -> String {
    match (
        store.filtered_seq(View::All, 0),
        store.last_filtered(View::All),
    ) {
        (Some(a), Some(b)) => range_text(store, View::All, a, b, clock),
        _ => String::new(),
    }
}
