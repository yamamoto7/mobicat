//! Screen snapshots: render whole frames into a `TestBackend` and compare.

use std::sync::{Arc, Mutex};

use jiff::tz::TimeZone;
use mobicat_core::{Clock, Entry, Level, Platform, Source, Store};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyModifiers};

use super::{App, draw, handle_key};

const T0: i64 = 1_794_057_430_000_000_000; // 2026-11-07T13:17:10Z

fn log(platform: Platform, ms: i64, level: Level, tag: &str, msg: &str, app_only: bool) -> Entry {
    let mut e = Entry::system(
        platform,
        Arc::from(if platform == Platform::Android {
            "emulator-5554"
        } else {
            "SIM-1"
        }),
        Some(Arc::from("com.example.app")),
        T0 + ms * 1_000_000,
        level,
        msg,
    );
    e.source = Source::Log;
    e.tag = Arc::from(tag);
    e.pid = if platform == Platform::Android {
        3990
    } else {
        80248
    };
    e.tid = e.pid as u64;
    e.app_only = app_only;
    if platform == Platform::Ios {
        e.subsystem = Some(Arc::from("com.example.app"));
        e.category = Some(Arc::from(tag));
        e.library = Some(Arc::from("ExampleApp.debug.dylib"));
    }
    e
}

fn crash(mut e: Entry) -> Entry {
    e.crash = true;
    e
}

fn sample_store() -> Store {
    use Level::*;
    use Platform::*;
    let mut s = Store::new(1000);
    s.add_device("emulator-5554", Android, "Pixel 9");
    s.add_device("SIM-1", Ios, "iPhone 17 Pro");
    s.push_batch([
        log(Android, 0, Info, "MainActivity", "onCreate", true),
        log(Ios, 7, Info, "network", "GET /api/items 200 (124 ms)", true),
        log(Ios, 8, Debug, "persistence", "saved 20 items", true),
        log(Ios, 9, Debug, "connection", "nw_connection_create [C1]", false),
        log(Android, 879, Error, "OkHttp", "timeout after 10s", true),
        crash(log(
            Android,
            1200,
            Fatal,
            "AndroidRuntime",
            "FATAL EXCEPTION: main\nProcess: com.example.app, PID: 3990\njava.lang.IllegalStateException: boom\n\tat com.example.app.MainActivity.onClick(MainActivity.kt:42)",
            true,
        )),
        Entry::system(Android, Arc::from("emulator-5554"), None, T0 + 2_000_000_000, Info, "process started (pid 4120)"),
        crash(log(Ios, 2410, Fatal, "libswiftCore.dylib", "ItemStore.swift:42: Fatal error: Unexpectedly found nil", true)),
    ]);
    s
}

fn app(store: Store) -> App {
    let opts = crate::TuiOptions {
        title: "com.example.app".into(),
        profiles: vec![("errors".into(), "level:error".into())],
        ..Default::default()
    };
    App::new(
        Arc::new(Mutex::new(store)),
        &opts,
        Clock::with_tz(TimeZone::UTC),
    )
}

fn screen(app: &mut App, w: u16, h: u16) -> String {
    let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
    t.draw(|f| draw(f, app)).unwrap();
    t.backend().to_string()
}

fn keys(app: &mut App, keys: &[KeyCode]) {
    for &k in keys {
        handle_key(app, k, KeyModifiers::NONE);
    }
}

fn ctrl(app: &mut App, c: char) {
    handle_key(app, KeyCode::Char(c), KeyModifiers::CONTROL);
}

#[test]
fn follow_view() {
    let mut a = app(sample_store());
    insta::assert_snapshot!(screen(&mut a, 110, 12));
}

#[test]
fn all_lines_and_paused_cursor() {
    let mut a = app(sample_store());
    keys(
        &mut a,
        &[KeyCode::Char('a'), KeyCode::Char('k'), KeyCode::Char('k')],
    );
    insta::assert_snapshot!(screen(&mut a, 110, 13));
}

#[test]
fn o_folds_an_open_crash_block() {
    let mut a = app(sample_store());
    keys(
        &mut a,
        &[
            KeyCode::Char('k'),
            KeyCode::Char('k'),
            KeyCode::Char('k'),
            KeyCode::Char('o'),
        ],
    );
    insta::assert_snapshot!(screen(&mut a, 110, 14));
}

#[test]
fn detail_pane_splits_tall_terminals() {
    let mut a = app(sample_store());
    keys(&mut a, &[KeyCode::Char('k'), KeyCode::Enter]);
    insta::assert_snapshot!(screen(&mut a, 100, 30));
}

#[test]
fn detail_pane_is_full_screen_on_short_terminals() {
    let mut a = app(sample_store());
    keys(
        &mut a,
        &[
            KeyCode::Char('k'),
            KeyCode::Char('k'),
            KeyCode::Char('k'),
            KeyCode::Enter,
        ],
    );
    insta::assert_snapshot!(screen(&mut a, 100, 16));
    keys(&mut a, &[KeyCode::Esc]);
    assert!(!a.detail);
    assert!(!a.quit, "Esc closes the detail pane before quitting");
}

#[test]
fn esc_and_q_go_back_then_q_arms_quit() {
    let mut a = app(sample_store());
    keys(&mut a, &[KeyCode::Char('k'), KeyCode::Enter]);
    assert!(a.detail);
    keys(&mut a, &[KeyCode::Char('q')]);
    assert!(!a.detail, "q closes the detail pane");
    assert!(a.vp().cursor().is_some(), "still paused");
    keys(&mut a, &[KeyCode::Esc]);
    assert!(a.vp().cursor().is_none(), "Esc resumes following");
    keys(&mut a, &[KeyCode::Esc]);
    assert!(!a.quit && a.quit_armed.is_none(), "Esc never quits");
    keys(&mut a, &[KeyCode::Char('q')]);
    assert!(!a.quit);
    assert!(screen(&mut a, 110, 12).contains("Press q again to exit"));
    ctrl(&mut a, 'c');
    assert!(a.quit, "a second press within the window quits");
}

#[test]
fn ctrl_c_or_ctrl_d_twice_quits() {
    for c in ['c', 'd'] {
        let mut a = app(sample_store());
        ctrl(&mut a, c);
        assert!(!a.quit);
        let key = if c == 'c' { "Ctrl-C" } else { "Ctrl-D" };
        assert!(screen(&mut a, 110, 12).contains(&format!("Press {key} again to exit")));
        ctrl(&mut a, c);
        assert!(a.quit);
    }
}

#[test]
fn clear_asks_first() {
    let mut a = app(sample_store());
    keys(&mut a, &[KeyCode::Char('c')]);
    assert!(screen(&mut a, 110, 12).contains("Clear the screen and the buffer?"));
    keys(&mut a, &[KeyCode::Char('n')]);
    assert!(shown(&a) > 0, "n keeps everything");
    keys(&mut a, &[KeyCode::Char('c'), KeyCode::Char('y')]);
    assert_eq!(shown(&a), 0);
    assert!(screen(&mut a, 110, 12).contains("✓ cleared"));
}

#[test]
fn which_key_after_bracket() {
    let mut a = app(sample_store());
    keys(&mut a, &[KeyCode::Char(']')]);
    insta::assert_snapshot!(screen(&mut a, 90, 14));
    keys(&mut a, &[KeyCode::Esc]);
    assert!(a.pending.is_none() && !a.quit);
}

#[test]
fn help_overlay() {
    let mut a = app(sample_store());
    keys(&mut a, &[KeyCode::Char('?')]);
    insta::assert_snapshot!(screen(&mut a, 100, 24));
    keys(&mut a, &[KeyCode::Char('x')]);
    assert!(!a.help && !a.quit);
}

#[test]
fn tab_cycles_targets_and_keeps_positions() {
    use mobicat_core::View;
    let mut a = app(sample_store());
    keys(&mut a, &[KeyCode::Tab]);
    assert_eq!(a.target, View::Device(0));
    let s = screen(&mut a, 110, 12);
    assert!(s.contains("MainActivity") && !s.contains("network"), "{s}");
    keys(&mut a, &[KeyCode::Char('k')]);
    let paused_at = a.vp().cursor();
    assert!(paused_at.is_some());
    keys(&mut a, &[KeyCode::Tab]);
    assert_eq!(a.target, View::Device(1));
    assert!(
        a.vp().cursor().is_none(),
        "each target has its own position"
    );
    keys(&mut a, &[KeyCode::BackTab]);
    assert_eq!(a.vp().cursor(), paused_at, "coming back restores it");
    keys(&mut a, &[KeyCode::Tab, KeyCode::Tab]);
    assert_eq!(a.target, View::All);
}

#[test]
fn unseen_crash_badge_and_toast() {
    use mobicat_core::View;
    let mut a = app(sample_store());
    screen(&mut a, 110, 12); // everything on screen counts as seen
    keys(&mut a, &[KeyCode::Tab]); // Pixel 9 only
    screen(&mut a, 110, 12);
    a.lock().push_batch([crash(log(
        Platform::Ios,
        3000,
        Level::Fatal,
        "libswiftCore.dylib",
        "Fatal error: again",
        true,
    ))]);
    let s = screen(&mut a, 110, 12);
    assert!(s.contains("iPhone 17 Pro 4 ✖1"), "{s}");
    assert!(s.contains("new crash on iPhone 17 Pro"), "{s}");
    keys(&mut a, &[KeyCode::Tab]);
    assert_eq!(a.target, View::Device(1));
    screen(&mut a, 110, 12);
    keys(&mut a, &[KeyCode::BackTab]);
    assert!(!screen(&mut a, 110, 12).contains("✖1"), "seen now");
}

#[test]
fn target_panel() {
    use mobicat_core::View;
    let mut a = app(sample_store());
    keys(&mut a, &[KeyCode::Char('d')]);
    insta::assert_snapshot!(screen(&mut a, 90, 16));
    // All → Android → Pixel 9 → iOS
    keys(
        &mut a,
        &[KeyCode::Down, KeyCode::Down, KeyCode::Down, KeyCode::Enter],
    );
    assert_eq!(a.target, View::Only(Platform::Ios));
    let s = screen(&mut a, 110, 12);
    assert!(s.contains("network") && !s.contains("MainActivity"), "{s}");
    keys(&mut a, &[KeyCode::Char('d')]);
    typed(&mut a, "pix");
    keys(&mut a, &[KeyCode::Down, KeyCode::Enter]);
    assert_eq!(
        a.target,
        View::Device(0),
        "typing narrows to Android ▸ Pixel 9"
    );
}

fn typed(app: &mut App, text: &str) {
    for c in text.chars() {
        handle_key(app, KeyCode::Char(c), KeyModifiers::NONE);
    }
}

fn shown(app: &App) -> usize {
    app.lock().filtered_len(mobicat_core::View::All)
}

#[test]
fn query_applies_live_and_esc_restores() {
    let mut a = app(sample_store());
    let before = shown(&a);
    keys(&mut a, &[KeyCode::Char('/')]);
    typed(&mut a, "tag:OkHttp");
    // Only the OkHttp line, plus mobicat's own process-started note.
    assert_eq!(shown(&a), 2);
    let s = screen(&mut a, 110, 12);
    assert!(s.contains("/ tag:OkHttp"), "{s}");
    keys(&mut a, &[KeyCode::Esc]);
    assert_eq!(shown(&a), before);
    assert!(a.query_text.is_empty());

    keys(&mut a, &[KeyCode::Char('/')]);
    typed(&mut a, "level:error");
    keys(&mut a, &[KeyCode::Enter]);
    assert_eq!(a.query_text, "level:error");
    assert!(screen(&mut a, 110, 12).contains("q: level:error"));
}

#[test]
fn query_errors_are_shown_and_block_enter() {
    let mut a = app(sample_store());
    keys(&mut a, &[KeyCode::Char('/')]);
    typed(&mut a, "level:loud");
    let s = screen(&mut a, 120, 12);
    assert!(s.contains("unknown level"), "{s}");
    keys(&mut a, &[KeyCode::Enter]);
    assert!(
        matches!(a.mode, crate::Mode::Input(_)),
        "Enter keeps editing an invalid query"
    );
}

#[test]
fn input_box_completes_and_remembers() {
    let mut a = app(sample_store());
    keys(&mut a, &[KeyCode::Char('/')]);
    typed(&mut a, "tag:ok");
    insta::assert_snapshot!(screen(&mut a, 90, 14));
    keys(&mut a, &[KeyCode::Tab]);
    assert_eq!(
        a.query_text, "tag:OkHttp",
        "Tab takes the only candidate and applies it"
    );
    typed(&mut a, " lev");
    keys(&mut a, &[KeyCode::Tab]);
    keys(&mut a, &[KeyCode::Char('e')]);
    keys(&mut a, &[KeyCode::Tab]);
    assert_eq!(a.query_text, "tag:OkHttp level:error");
    keys(&mut a, &[KeyCode::Enter]);
    assert_eq!(a.history, ["tag:OkHttp level:error"]);

    keys(&mut a, &[KeyCode::Char('/')]);
    ctrl(&mut a, 'u');
    keys(&mut a, &[KeyCode::Up]);
    assert_eq!(
        a.query_text, "tag:OkHttp level:error",
        "↑ brings back the last query"
    );
    keys(&mut a, &[KeyCode::Down]);
    assert_eq!(
        a.query_text, "",
        "↓ past the newest returns to what was typed"
    );
    keys(&mut a, &[KeyCode::Esc]);
}

#[test]
fn tab_cycles_several_candidates() {
    let mut a = app(sample_store());
    keys(&mut a, &[KeyCode::Char('/')]);
    typed(&mut a, "level:");
    keys(&mut a, &[KeyCode::Tab]);
    assert_eq!(a.query_text, "level:verbose");
    keys(&mut a, &[KeyCode::Tab, KeyCode::Tab]);
    assert_eq!(a.query_text, "level:info");
    keys(&mut a, &[KeyCode::BackTab]);
    assert_eq!(a.query_text, "level:debug");
}

#[test]
fn palette_runs_actions_by_name() {
    let mut a = app(sample_store());
    keys(&mut a, &[KeyCode::Char(':')]);
    typed(&mut a, "prev crash");
    insta::assert_snapshot!(screen(&mut a, 90, 12));
    keys(&mut a, &[KeyCode::Enter]);
    let tag = {
        let c = a.vp().cursor();
        c.and_then(|s| a.lock().get(s).map(|e| e.tag.to_string()))
    };
    assert_eq!(tag.as_deref(), Some("libswiftCore.dylib"));
    keys(&mut a, &[KeyCode::Char(':')]);
    typed(&mut a, "change app");
    keys(&mut a, &[KeyCode::Enter]);
    assert!(
        !a.stats.change_app,
        "not offered unless the caller supports it"
    );
    a.can_change_app = true;
    keys(&mut a, &[KeyCode::Char(':')]);
    typed(&mut a, "change app");
    keys(&mut a, &[KeyCode::Enter]);
    assert!(a.stats.change_app && a.quit);
}

#[test]
fn crash_navigation_and_count() {
    let mut a = app(sample_store());
    assert!(screen(&mut a, 110, 12).contains("✖ 2"));
    keys(&mut a, &[KeyCode::Char('['), KeyCode::Char('c')]);
    let cur = |a: &mut App| {
        let c = a.vp().cursor();
        c.and_then(|s| a.lock().get(s).map(|e| e.tag.to_string()))
    };
    assert_eq!(cur(&mut a).as_deref(), Some("libswiftCore.dylib"));
    keys(&mut a, &[KeyCode::Char('['), KeyCode::Char('c')]);
    assert_eq!(cur(&mut a).as_deref(), Some("AndroidRuntime"));
    keys(&mut a, &[KeyCode::Char('['), KeyCode::Char('c')]);
    assert!(a.toast.as_ref().is_some_and(|t| t.text == "no older crash"));
    keys(&mut a, &[KeyCode::Char('e')]);
    assert_eq!(
        cur(&mut a).as_deref(),
        Some("libswiftCore.dylib"),
        "e = next error or worse"
    );
}

#[test]
fn highlight_mode_keeps_everything() {
    let mut a = app(sample_store());
    let all = shown(&a);
    keys(&mut a, &[KeyCode::Char('/')]);
    typed(&mut a, "network");
    keys(&mut a, &[KeyCode::Enter, KeyCode::Char('x')]);
    assert_eq!(shown(&a), all);
    assert!(screen(&mut a, 110, 12).contains("highlight: network"));
}

#[test]
fn picker_adds_a_term() {
    let mut a = app(sample_store());
    keys(&mut a, &[KeyCode::Char('t')]);
    typed(&mut a, "okht");
    let s = screen(&mut a, 110, 24);
    assert!(s.contains("OkHttp"), "{s}");
    keys(&mut a, &[KeyCode::Enter]);
    assert_eq!(a.query_text, "tag:OkHttp");
    keys(&mut a, &[KeyCode::Char('t'), KeyCode::Char('-')]);
    assert!(a.query_text.starts_with("tag:OkHttp -"), "{}", a.query_text);
}

#[test]
fn tag_from_cursor_and_profiles() {
    let mut a = app(sample_store());
    // newest (iOS crash) → mobicat's note (no tag; ignored) → the Android crash
    keys(
        &mut a,
        &[KeyCode::Char('k'), KeyCode::Char('k'), KeyCode::Char('!')],
    );
    assert_eq!(
        a.query_text, "",
        "mobicat's own lines have no tag to filter by"
    );
    keys(&mut a, &[KeyCode::Char('k'), KeyCode::Char('!')]);
    assert_eq!(a.query_text, "-tag:AndroidRuntime");
    keys(&mut a, &[KeyCode::Char('1')]);
    assert_eq!(a.query_text, "level:error");
    keys(&mut a, &[KeyCode::Char('2')]);
    assert!(
        a.toast
            .as_ref()
            .is_some_and(|t| t.text.contains("no profile 2"))
    );
}

#[test]
fn range_text_is_plain() {
    let a = app(sample_store());
    let store = a.lock();
    let text = crate::keys::range_text_for_test(&store, &a.clock);
    assert!(
        text.starts_with("13:17:10.000 A I         MainActivity onCreate\n"),
        "{text}"
    );
    assert!(text.contains("✖ CRASH  FATAL EXCEPTION: main\nProcess:"));
    assert!(!text.contains('\x1b'), "no color codes");
}
