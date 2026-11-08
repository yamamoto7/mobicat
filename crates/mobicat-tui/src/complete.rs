//! Query completion: key names (`tag:`, `level:` …) and values, including the
//! tags and subsystems actually in the buffer.

use std::collections::HashMap;

use mobicat_core::query::FIELDS;
use mobicat_core::{Source, Store, View};

/// How many recent entries to scan for tag-like values.
const SCAN: usize = 20_000;
const MAX: usize = 8;

/// Where the token being completed starts in `text`, and the candidates that
/// would replace it.
pub fn suggest(text: &str, store: &Store, view: View) -> (usize, Vec<String>) {
    let start = text
        .rfind(|c: char| c.is_whitespace() || c == '(')
        .map_or(0, |i| i + 1);
    let token = &text[start..];
    let (neg, token) = match token.strip_prefix('-') {
        Some(t) => ("-", t),
        None => ("", token),
    };
    let Some((key, value)) = token.split_once(':') else {
        let lower = token.to_lowercase();
        let keys = FIELDS
            .iter()
            .filter(|k| k.starts_with(&lower) && **k != lower)
            .map(|k| format!("{neg}{k}:"))
            .take(MAX)
            .collect();
        return (start, keys);
    };
    // `tag~:` and `tag=:` complete like `tag:`.
    let field = key.trim_end_matches(['~', '=']);
    let fixed: &[&str] = match field {
        "level" => &["verbose", "debug", "info", "warn", "error", "fatal"],
        "is" => &["crash", "stacktrace", "stdout", "system"],
        "fw" | "framework" => &["native", "react-native", "flutter", "kmp", "capacitor"],
        "os" => &["android", "ios"],
        "age" => &["30s", "1m", "5m", "1h"],
        _ => &[],
    };
    let lower = value.to_lowercase();
    let values: Vec<String> = if fixed.is_empty() {
        frequent(store, view, field)
            .into_iter()
            .filter(|v| v.to_lowercase().contains(&lower) && *v != value)
            .collect()
    } else {
        fixed
            .iter()
            .filter(|v| v.starts_with(&lower) && **v != lower)
            .map(|v| (*v).to_owned())
            .collect()
    };
    let out = values
        .into_iter()
        .take(MAX)
        .map(|v| format!("{neg}{key}:{}", quote(&v)))
        .collect();
    (start, out)
}

/// Values of a text field in the newest entries, most frequent first.
fn frequent(store: &Store, view: View, field: &str) -> Vec<String> {
    let n = store.filtered_len(view);
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for i in n.saturating_sub(SCAN)..n {
        let Some(e) = store.filtered_seq(view, i).and_then(|s| store.get(s)) else {
            continue;
        };
        if e.source == Source::System {
            continue;
        }
        let v: Option<&str> = match field {
            "tag" => Some(&e.tag),
            "subsystem" => e.subsystem.as_deref(),
            "category" => e.category.as_deref(),
            "library" => e.library.as_deref(),
            "package" | "app" => e.app.as_deref(),
            "device" => Some(&e.device),
            _ => None,
        };
        if let Some(v) = v {
            *counts.entry(v).or_default() += 1;
        }
    }
    let mut v: Vec<(&str, usize)> = counts.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    v.into_iter().map(|(s, _)| s.to_owned()).collect()
}

fn quote(v: &str) -> String {
    if v.is_empty() || v.chars().any(|c| c.is_whitespace() || "()|&\"".contains(c)) {
        format!("\"{}\"", v.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        v.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use mobicat_core::{Entry, Level, Platform};

    use super::*;

    fn store() -> Store {
        let mut s = Store::new(10);
        let e = |tag: &str| {
            let mut e = Entry::system(Platform::Android, Arc::from("d"), None, 0, Level::Info, "m");
            e.source = Source::Log;
            e.tag = Arc::from(tag);
            e
        };
        s.push_batch([e("OkHttp"), e("OkHttp"), e("MainActivity"), e("Okio")]);
        s
    }

    #[test]
    fn keys_values_and_tags() {
        let s = store();
        let v = View::All;
        assert_eq!(suggest("le", &s, v), (0, vec!["level:".to_owned()]));
        assert_eq!(
            suggest("tag:x level:w", &s, v),
            (6, vec!["level:warn".to_owned()])
        );
        assert_eq!(
            suggest("-tag:ok", &s, v).1,
            ["-tag:OkHttp", "-tag:Okio"],
            "tags in the buffer, most frequent first"
        );
        assert!(
            suggest("tag:OkHttp", &s, v).1.is_empty(),
            "already complete"
        );
        assert_eq!(suggest("", &s, v).1.len(), MAX);
    }
}
