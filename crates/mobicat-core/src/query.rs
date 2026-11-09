//! The filter query language (docs/requirements.md §7).
//!
//! Based on Android Studio Logcat's: `key:value` (substring), `key~:regex`,
//! `-key:value` (exclude), bare words (message), `level:warn` (at least),
//! `is:crash`, `age:5m`, plus `subsystem:`, `category:`, `library:`, `os:`,
//! `fw:` for iOS and cross-platform use.
//!
//! Combining: within a run of terms, repeated fields are ORed and different
//! fields are ANDed (`tag:a tag:b level:error` = `(tag:a | tag:b) & level:error`).
//! Bare words are always ANDed (`http failed` needs both words).
//! `|`, `&` and parentheses are explicit; `&` binds tighter than `|`.
//! Text matches are case-insensitive unless the value has an uppercase letter.

use std::fmt;

use regex::{Regex, RegexBuilder};

use crate::entry::{Entry, Framework, Level, Platform, Source};

#[derive(Clone, Debug)]
pub struct Query {
    source: String,
    expr: Expr,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueryError {
    /// Byte offset in the query string.
    pub pos: usize,
    pub message: String,
}

impl fmt::Display for QueryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} (at column {})", self.message, self.pos + 1)
    }
}

impl std::error::Error for QueryError {}

#[derive(Clone, Debug)]
enum Expr {
    True,
    And(Vec<Expr>),
    Or(Vec<Expr>),
    Not(Box<Expr>),
    Term(Term),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Field {
    Tag,
    Message,
    Subsystem,
    Category,
    Library,
    Package,
    Device,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Is {
    Crash,
    Stacktrace,
    Stdout,
    System,
}

#[derive(Clone, Debug)]
enum Term {
    Text(Field, Regex),
    Level(Level),
    Pid(u32),
    Os(Platform),
    Is(Is),
    Fw(Framework),
    /// Maximum age in nanoseconds.
    Age(i64),
}

pub const FIELDS: &[&str] = &[
    "tag",
    "message",
    "subsystem",
    "category",
    "library",
    "package",
    "device",
    "level",
    "pid",
    "os",
    "is",
    "fw",
    "age",
];

impl Query {
    pub fn parse(source: &str) -> Result<Query, QueryError> {
        let tokens = lex(source)?;
        let mut p = Parser { tokens, i: 0 };
        let expr = if p.tokens.is_empty() {
            Expr::True
        } else {
            let e = p.expr()?;
            if let Some(t) = p.tokens.get(p.i) {
                return Err(QueryError {
                    pos: t.pos,
                    message: "unexpected `)`".into(),
                });
            }
            e
        };
        Ok(Query {
            source: source.to_owned(),
            expr,
        })
    }

    pub fn as_str(&self) -> &str {
        &self.source
    }

    pub fn is_empty(&self) -> bool {
        matches!(self.expr, Expr::True)
    }

    /// `now_ns` is used by `age:`.
    pub fn matches(&self, e: &Entry, now_ns: i64) -> bool {
        eval(&self.expr, e, now_ns)
    }
}

fn eval(x: &Expr, e: &Entry, now: i64) -> bool {
    match x {
        Expr::True => true,
        Expr::And(v) => v.iter().all(|x| eval(x, e, now)),
        Expr::Or(v) => v.iter().any(|x| eval(x, e, now)),
        Expr::Not(x) => !eval(x, e, now),
        Expr::Term(t) => term(t, e, now),
    }
}

fn term(t: &Term, e: &Entry, now: i64) -> bool {
    match t {
        Term::Text(field, re) => {
            let v: Option<&str> = match field {
                Field::Tag => Some(&e.tag),
                Field::Message => Some(&e.message),
                Field::Subsystem => e.subsystem.as_deref(),
                Field::Category => e.category.as_deref(),
                Field::Library => e.library.as_deref(),
                Field::Package => e.app.as_deref(),
                Field::Device => Some(&e.device),
            };
            v.is_some_and(|v| re.is_match(v))
        }
        Term::Level(l) => e.level >= *l,
        Term::Pid(p) => e.pid == *p,
        Term::Os(p) => e.platform == *p,
        Term::Is(Is::Crash) => e.crash,
        Term::Is(Is::Stacktrace) => is_stacktrace(&e.message),
        Term::Is(Is::Stdout) => matches!(e.source, Source::Stdout | Source::Stderr),
        Term::Is(Is::System) => e.source == Source::System,
        Term::Fw(f) => e.framework == *f,
        Term::Age(max) => now - e.time_ns <= *max,
    }
}

fn is_stacktrace(m: &str) -> bool {
    m.contains("\n\tat ")
        || m.contains("\n    at ")
        || m.contains("\nCaused by: ")
        || m.contains("\n#00 pc ")
        || m.contains("Thread 0 Crashed")
}

// --- lexing ---------------------------------------------------------------

#[derive(Debug)]
struct Token {
    pos: usize,
    kind: Tok,
}

#[derive(Debug)]
enum Tok {
    LParen { negated: bool },
    RParen,
    Or,
    And,
    Term(RawTerm),
}

#[derive(Debug)]
struct RawTerm {
    negated: bool,
    key: Option<String>,
    regex: bool,
    value: String,
    pos: usize,
}

fn lex(s: &str) -> Result<Vec<Token>, QueryError> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        let pos = i;
        match c {
            b'(' => {
                out.push(Token {
                    pos,
                    kind: Tok::LParen { negated: false },
                });
                i += 1;
            }
            b')' => {
                out.push(Token {
                    pos,
                    kind: Tok::RParen,
                });
                i += 1;
            }
            b'|' => {
                out.push(Token { pos, kind: Tok::Or });
                i += 1;
            }
            b'&' => {
                out.push(Token {
                    pos,
                    kind: Tok::And,
                });
                i += 1;
            }
            b'-' if b.get(i + 1) == Some(&b'(') => {
                out.push(Token {
                    pos,
                    kind: Tok::LParen { negated: true },
                });
                i += 2;
            }
            _ => {
                let (t, next) = lex_term(s, i)?;
                out.push(Token {
                    pos,
                    kind: Tok::Term(t),
                });
                i = next;
            }
        }
    }
    Ok(out)
}

fn lex_term(s: &str, start: usize) -> Result<(RawTerm, usize), QueryError> {
    let b = s.as_bytes();
    let mut i = start;
    let negated = b[i] == b'-' && i + 1 < b.len() && !b[i + 1].is_ascii_whitespace();
    if negated {
        i += 1;
    }
    // key:  or  key~:
    let mut key = None;
    let mut regex = false;
    let key_start = i;
    let mut j = i;
    while j < b.len() && (b[j].is_ascii_alphabetic()) {
        j += 1;
    }
    if j > key_start {
        if b.get(j) == Some(&b':') {
            key = Some(s[key_start..j].to_ascii_lowercase());
            i = j + 1;
        } else if b.get(j) == Some(&b'~') && b.get(j + 1) == Some(&b':') {
            key = Some(s[key_start..j].to_ascii_lowercase());
            regex = true;
            i = j + 2;
        }
    }
    let value_pos = i;
    let (value, next) = if b.get(i) == Some(&b'"') {
        let mut v = String::new();
        let mut k = i + 1;
        loop {
            match b.get(k) {
                None => {
                    return Err(QueryError {
                        pos: i,
                        message: "unterminated quote".into(),
                    });
                }
                Some(b'"') => break,
                Some(b'\\') if matches!(b.get(k + 1), Some(b'"') | Some(b'\\')) => {
                    v.push(b[k + 1] as char);
                    k += 2;
                }
                Some(_) => {
                    let ch = s[k..].chars().next().unwrap_or('\0');
                    v.push(ch);
                    k += ch.len_utf8();
                }
            }
        }
        (v, k + 1)
    } else {
        // Unquoted: up to whitespace. Plain values also stop at `)` `|` `&`;
        // regex values keep balanced parentheses.
        let mut k = i;
        let mut depth = 0_i32;
        while k < b.len() {
            let c = b[k];
            if c.is_ascii_whitespace() {
                break;
            }
            if regex {
                match c {
                    b'(' => depth += 1,
                    b')' if depth == 0 => break,
                    b')' => depth -= 1,
                    _ => {}
                }
            } else if matches!(c, b'(' | b')' | b'|' | b'&') {
                break;
            }
            k += 1;
        }
        (s[i..k].to_owned(), k)
    };
    if value.is_empty() {
        return Err(QueryError {
            pos: value_pos,
            message: match &key {
                Some(k) => format!("`{k}:` needs a value"),
                None => "expected a term".into(),
            },
        });
    }
    Ok((
        RawTerm {
            negated,
            key,
            regex,
            value,
            pos: start,
        },
        next,
    ))
}

// --- parsing --------------------------------------------------------------

struct Parser {
    tokens: Vec<Token>,
    i: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.tokens.get(self.i).map(|t| &t.kind)
    }

    fn end_pos(&self) -> usize {
        self.tokens.last().map_or(0, |t| t.pos + 1)
    }

    fn expr(&mut self) -> Result<Expr, QueryError> {
        let mut parts = vec![self.and()?];
        while matches!(self.peek(), Some(Tok::Or)) {
            self.i += 1;
            parts.push(self.and()?);
        }
        Ok(if parts.len() == 1 {
            parts.pop().unwrap_or(Expr::True)
        } else {
            Expr::Or(parts)
        })
    }

    fn and(&mut self) -> Result<Expr, QueryError> {
        let mut parts = vec![self.seq()?];
        while matches!(self.peek(), Some(Tok::And)) {
            self.i += 1;
            parts.push(self.seq()?);
        }
        Ok(if parts.len() == 1 {
            parts.pop().unwrap_or(Expr::True)
        } else {
            Expr::And(parts)
        })
    }

    /// A run of adjacent atoms, combined with the Android Studio rule.
    fn seq(&mut self) -> Result<Expr, QueryError> {
        let mut groups: Vec<(String, Vec<Expr>)> = Vec::new();
        let mut others: Vec<Expr> = Vec::new();
        loop {
            match self.peek() {
                None | Some(Tok::RParen) | Some(Tok::Or) | Some(Tok::And) => break,
                Some(Tok::LParen { negated }) => {
                    let negated = *negated;
                    let open = self.tokens[self.i].pos;
                    self.i += 1;
                    let inner = self.expr()?;
                    if !matches!(self.peek(), Some(Tok::RParen)) {
                        return Err(QueryError {
                            pos: open,
                            message: "unclosed `(`".into(),
                        });
                    }
                    self.i += 1;
                    others.push(if negated {
                        Expr::Not(Box::new(inner))
                    } else {
                        inner
                    });
                }
                Some(Tok::Term(_)) => {
                    let Tok::Term(raw) =
                        std::mem::replace(&mut self.tokens[self.i].kind, Tok::RParen)
                    else {
                        unreachable!()
                    };
                    self.i += 1;
                    // Bare words always AND ("http failed" means both words);
                    // repeated `key:` terms OR.
                    let key = raw.key.clone().unwrap_or_else(|| format!("\0{}", raw.pos));
                    let negated = raw.negated;
                    let t = Expr::Term(compile(raw)?);
                    if negated {
                        others.push(Expr::Not(Box::new(t)));
                    } else if let Some((_, g)) = groups.iter_mut().find(|(k, _)| *k == key) {
                        g.push(t);
                    } else {
                        groups.push((key, vec![t]));
                    }
                }
            }
        }
        if groups.is_empty() && others.is_empty() {
            let pos = self.tokens.get(self.i).map_or(self.end_pos(), |t| t.pos);
            return Err(QueryError {
                pos,
                message: "expected a term".into(),
            });
        }
        let mut all: Vec<Expr> = groups
            .into_iter()
            .map(|(_, mut g)| {
                if g.len() == 1 {
                    g.pop().unwrap_or(Expr::True)
                } else {
                    Expr::Or(g)
                }
            })
            .collect();
        all.extend(others);
        Ok(if all.len() == 1 {
            all.pop().unwrap_or(Expr::True)
        } else {
            Expr::And(all)
        })
    }
}

fn text_regex(raw: &RawTerm) -> Result<Regex, QueryError> {
    let smart_case = !raw.value.chars().any(char::is_uppercase);
    let pattern = if raw.regex {
        raw.value.clone()
    } else {
        regex::escape(&raw.value)
    };
    RegexBuilder::new(&pattern)
        .case_insensitive(smart_case && !raw.regex)
        .size_limit(1 << 20)
        .build()
        .map_err(|e| QueryError {
            pos: raw.pos,
            message: format!(
                "invalid regex: {}",
                e.to_string().lines().last().unwrap_or("")
            ),
        })
}

fn compile(raw: RawTerm) -> Result<Term, QueryError> {
    let err = |m: String| QueryError {
        pos: raw.pos,
        message: m,
    };
    let key = raw.key.as_deref().unwrap_or("message");
    let v = raw.value.to_ascii_lowercase();
    let no_regex = |k: &str| -> Result<(), QueryError> {
        if raw.regex {
            Err(QueryError {
                pos: raw.pos,
                message: format!("`{k}~:` does not take a regex"),
            })
        } else {
            Ok(())
        }
    };
    Ok(match key {
        "tag" => Term::Text(Field::Tag, text_regex(&raw)?),
        "message" | "msg" => Term::Text(Field::Message, text_regex(&raw)?),
        "subsystem" => Term::Text(Field::Subsystem, text_regex(&raw)?),
        "category" => Term::Text(Field::Category, text_regex(&raw)?),
        "library" | "lib" => Term::Text(Field::Library, text_regex(&raw)?),
        "package" | "app" => Term::Text(Field::Package, text_regex(&raw)?),
        "device" => Term::Text(Field::Device, text_regex(&raw)?),
        "level" => {
            no_regex(key)?;
            Term::Level(match v.as_str() {
                "v" | "verbose" => Level::Verbose,
                "d" | "debug" => Level::Debug,
                "i" | "info" | "default" => Level::Info,
                "w" | "warn" | "warning" => Level::Warn,
                "e" | "error" => Level::Error,
                "f" | "fatal" | "a" | "assert" | "fault" => Level::Fatal,
                _ => {
                    return Err(err(format!(
                        "unknown level `{}` (verbose, debug, info, warn, error, fatal)",
                        raw.value
                    )));
                }
            })
        }
        "pid" => {
            no_regex(key)?;
            Term::Pid(
                v.parse()
                    .map_err(|_| err(format!("`pid:` needs a number, not `{}`", raw.value)))?,
            )
        }
        "os" => {
            no_regex(key)?;
            Term::Os(match v.as_str() {
                "android" | "a" => Platform::Android,
                "ios" | "i" => Platform::Ios,
                _ => return Err(err(format!("unknown os `{}` (android, ios)", raw.value))),
            })
        }
        "is" => {
            no_regex(key)?;
            Term::Is(match v.as_str() {
                "crash" => Is::Crash,
                "stacktrace" => Is::Stacktrace,
                "stdout" => Is::Stdout,
                "system" | "mobicat" => Is::System,
                _ => {
                    return Err(err(format!(
                        "unknown `is:{}` (crash, stacktrace, stdout, system)",
                        raw.value
                    )));
                }
            })
        }
        "fw" | "framework" => {
            no_regex(key)?;
            Term::Fw(Framework::parse(&v).ok_or_else(|| {
                err(format!(
                    "unknown framework `{}` (native, react-native, flutter, kmp, capacitor)",
                    raw.value
                ))
            })?)
        }
        "age" => {
            no_regex(key)?;
            let d = parse_duration(&v).ok_or_else(|| {
                err(format!(
                    "`age:` needs a duration like 30s, 5m, 1h, not `{}`",
                    raw.value
                ))
            })?;
            Term::Age(i64::try_from(d.as_nanos()).unwrap_or(i64::MAX))
        }
        other => {
            return Err(err(format!(
                "unknown field `{other}` (fields: {})",
                FIELDS.join(", ")
            )));
        }
    })
}

/// A duration like `30s`, `5m`, `1h` or `2d`. A bare number is seconds.
pub fn parse_duration(v: &str) -> Option<std::time::Duration> {
    let (num, unit) = v.split_at(v.find(|c: char| !c.is_ascii_digit()).unwrap_or(v.len()));
    let n: u64 = num.parse().ok()?;
    let secs = match unit {
        "s" | "" => n,
        "m" => n.checked_mul(60)?,
        "h" => n.checked_mul(3600)?,
        "d" => n.checked_mul(86_400)?,
        _ => return None,
    };
    Some(std::time::Duration::from_secs(secs))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    fn e(platform: Platform, level: Level, tag: &str, msg: &str) -> Entry {
        let mut e = Entry::system(
            platform,
            Arc::from("emulator-5554"),
            Some(Arc::from("com.example.app")),
            1_000,
            level,
            msg,
        );
        e.source = Source::Log;
        e.tag = Arc::from(tag);
        e.pid = 42;
        e
    }

    fn m(q: &str, entry: &Entry) -> bool {
        Query::parse(q)
            .unwrap_or_else(|err| panic!("{q}: {err}"))
            .matches(entry, 1_000)
    }

    fn sample() -> Vec<Entry> {
        let mut ios = e(Platform::Ios, Level::Info, "network", "GET /items 200");
        ios.subsystem = Some(Arc::from("com.example.app"));
        ios.category = Some(Arc::from("network"));
        let mut crash = e(
            Platform::Android,
            Level::Fatal,
            "AndroidRuntime",
            "FATAL EXCEPTION: main\n\tat a.b(C.kt:1)",
        );
        crash.crash = true;
        vec![
            e(
                Platform::Android,
                Level::Debug,
                "OkHttp",
                "--> GET https://api",
            ),
            e(
                Platform::Android,
                Level::Error,
                "OkHttp",
                "<-- HTTP FAILED: timeout",
            ),
            e(Platform::Android, Level::Info, "MainActivity", "onCreate"),
            ios,
            crash,
        ]
    }

    fn matching(q: &str) -> Vec<String> {
        let query = Query::parse(q).unwrap_or_else(|err| panic!("{q}: {err}"));
        sample()
            .iter()
            .filter(|x| query.matches(x, 1_000))
            .map(|x| x.message.lines().next().unwrap_or("").to_owned())
            .collect()
    }

    #[test]
    fn empty_matches_everything() {
        assert!(Query::parse("").unwrap().is_empty());
        assert!(Query::parse("   ").unwrap().is_empty());
        assert_eq!(matching("").len(), 5);
    }

    #[test]
    fn substrings_and_smart_case() {
        assert_eq!(matching("tag:okhttp").len(), 2);
        assert_eq!(matching("tag:OkHttp").len(), 2);
        assert_eq!(
            matching("tag:OKHTTP").len(),
            0,
            "uppercase makes it case-sensitive"
        );
        assert_eq!(matching("timeout"), vec!["<-- HTTP FAILED: timeout"]);
        assert_eq!(
            matching("\"HTTP FAILED\""),
            vec!["<-- HTTP FAILED: timeout"]
        );
    }

    #[test]
    fn android_studio_combination_rule() {
        // Same field ORs, different fields AND.
        assert_eq!(matching("tag:OkHttp tag:MainActivity").len(), 3);
        assert_eq!(
            matching("tag:OkHttp level:error"),
            vec!["<-- HTTP FAILED: timeout"]
        );
        assert_eq!(
            matching("tag:OkHttp -level:error"),
            vec!["--> GET https://api"],
            "-level:error = below error"
        );
        assert_eq!(
            matching("http failed"),
            vec!["<-- HTTP FAILED: timeout"],
            "bare words AND"
        );
        assert_eq!(
            matching("message:onCreate message:timeout").len(),
            2,
            "repeated message: ORs"
        );
        assert_eq!(matching("tag:OkHttp -timeout"), vec!["--> GET https://api"]);
    }

    #[test]
    fn explicit_operators_and_parentheses() {
        assert_eq!(matching("tag:MainActivity | level:fatal").len(), 2);
        assert_eq!(matching("os:ios | tag:OkHttp & level:e").len(), 2);
        assert_eq!(matching("(os:ios | tag:OkHttp) & level:e").len(), 1);
        assert_eq!(matching("-(tag:OkHttp)").len(), 3);
    }

    #[test]
    fn typed_fields() {
        assert_eq!(matching("level:warn").len(), 2);
        assert_eq!(matching("level:W").len(), 2);
        assert_eq!(matching("is:crash"), vec!["FATAL EXCEPTION: main"]);
        assert_eq!(matching("is:stacktrace"), vec!["FATAL EXCEPTION: main"]);
        assert_eq!(matching("os:ios"), vec!["GET /items 200"]);
        assert_eq!(matching("subsystem:com.example category:net").len(), 1);
        assert_eq!(matching("pid:42").len(), 5);
        assert_eq!(matching("package:example").len(), 5);
        assert_eq!(matching("fw:native").len(), 5);
    }

    #[test]
    fn regex_terms() {
        assert_eq!(matching("tag~:^Ok").len(), 2);
        assert_eq!(matching("message~:(GET|POST) https").len(), 1);
        assert_eq!(
            matching("(tag~:^(Ok|Main))").len(),
            3,
            "balanced parens stay in the regex"
        );
        assert_eq!(matching("-tag~:Http$").len(), 3);
    }

    #[test]
    fn age() {
        let mut old = e(Platform::Android, Level::Info, "T", "old");
        old.time_ns = 0;
        let q = Query::parse("age:1s").unwrap();
        assert!(q.matches(&old, 999_999_999));
        assert!(!q.matches(&old, 2_000_000_000));
        assert!(m("age:5m", &old));
    }

    #[test]
    fn durations() {
        use std::time::Duration;
        assert_eq!(parse_duration("30s"), Some(Duration::from_secs(30)));
        assert_eq!(parse_duration("90"), Some(Duration::from_secs(90)));
        assert_eq!(parse_duration("5m"), Some(Duration::from_secs(300)));
        assert_eq!(parse_duration("2h"), Some(Duration::from_secs(7200)));
        assert_eq!(parse_duration("1d"), Some(Duration::from_secs(86_400)));
        for bad in ["", "m", "5x", "-1m", "1.5m"] {
            assert_eq!(parse_duration(bad), None, "{bad}");
        }
    }

    #[test]
    fn errors_point_at_the_problem() {
        let err = |q: &str| Query::parse(q).unwrap_err();
        assert_eq!(err("level:loud").pos, 0);
        assert!(err("level:loud").message.contains("unknown level"));
        assert_eq!(err("tag:a nope:x").pos, 6);
        assert!(err("nope:x").message.contains("unknown field"));
        assert!(err("tag:").message.contains("needs a value"));
        assert!(err("(tag:a").message.contains("unclosed"));
        assert!(err("tag:a)").message.contains("unexpected"));
        assert!(err("tag~:(").message.contains("invalid regex"));
        assert!(err("\"open").message.contains("unterminated"));
        assert!(err("level~:e").message.contains("does not take a regex"));
        assert!(err("tag:a |").message.contains("expected a term"));
    }
}
