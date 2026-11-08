use jiff::Timestamp;
use jiff::tz::TimeZone;

/// Formats entry times in the local time zone.
#[derive(Clone)]
pub struct Clock {
    tz: TimeZone,
}

impl Default for Clock {
    fn default() -> Self {
        Self::system()
    }
}

impl Clock {
    pub fn system() -> Self {
        Self {
            tz: TimeZone::try_system().unwrap_or(TimeZone::UTC),
        }
    }

    pub fn with_tz(tz: TimeZone) -> Self {
        Self { tz }
    }

    pub fn tz(&self) -> &TimeZone {
        &self.tz
    }

    fn timestamp(time_ns: i64) -> Timestamp {
        Timestamp::from_nanosecond(i128::from(time_ns)).unwrap_or(Timestamp::UNIX_EPOCH)
    }

    /// `HH:MM:SS.mmm` in local time.
    pub fn hms_millis(&self, time_ns: i64) -> String {
        let ts = Self::timestamp(time_ns);
        let offset = self.tz.to_offset(ts);
        let local = ts.to_zoned(TimeZone::fixed(offset));
        format!(
            "{:02}:{:02}:{:02}.{:03}",
            local.hour(),
            local.minute(),
            local.second(),
            local.subsec_nanosecond() / 1_000_000
        )
    }

    /// RFC 3339 with the local UTC offset and microseconds, e.g.
    /// `2026-11-07T22:17:10.100001+09:00`.
    pub fn rfc3339(&self, time_ns: i64) -> String {
        let ts = Self::timestamp(time_ns);
        let offset = self.tz.to_offset(ts);
        let rounded = ts.round(jiff::Unit::Microsecond).unwrap_or(ts);
        format!("{:.6}", rounded.display_with_offset(offset))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokyo() -> Clock {
        Clock::with_tz(TimeZone::fixed(jiff::tz::offset(9)))
    }

    #[test]
    fn formats_local_time() {
        // 2026-11-07T13:17:10.100001Z
        let ns = 1_794_057_430_100_001_000;
        assert_eq!(tokyo().hms_millis(ns), "22:17:10.100");
        assert_eq!(tokyo().rfc3339(ns), "2026-11-07T22:17:10.100001+09:00");
    }
}
