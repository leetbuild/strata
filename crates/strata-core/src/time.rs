//! The local time, for backup names and trash records (`QDateTime::currentDateTime`).

/// A local date and time, to the second.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalTime {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
}

impl LocalTime {
    /// Now, in the local time zone (`localtime_r`; UTC if that fails).
    pub fn now() -> Self {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        Self::at(i64::try_from(now).unwrap_or(0))
    }

    /// The local time of `secs` since 1970 (UTC if the time zone can't be read).
    pub fn at(secs: i64) -> Self {
        let t = libc::time_t::try_from(secs).unwrap_or(0);
        // SAFETY: `tm` is plain data; `localtime_r` writes it and reads `t`.
        let mut tm: libc::tm = unsafe { std::mem::zeroed() };
        let ok = unsafe { !libc::localtime_r(&t, &mut tm).is_null() };
        if !ok {
            // SAFETY: as above.
            unsafe { libc::gmtime_r(&t, &mut tm) };
        }
        let field = |v: libc::c_int| u32::try_from(v).unwrap_or(0);
        Self {
            year: tm.tm_year + 1900,
            month: field(tm.tm_mon + 1),
            day: field(tm.tm_mday),
            hour: field(tm.tm_hour),
            minute: field(tm.tm_min),
            second: field(tm.tm_sec),
        }
    }

    /// `yyyy_MM_dd_hh_mm_ss`, MO2's backup suffix (`MainWindow::PATTERN_BACKUP_DATE`).
    pub fn backup_suffix(&self) -> String {
        format!("{:04}_{:02}_{:02}_{:02}_{:02}_{:02}", self.year, self.month, self.day, self.hour, self.minute, self.second)
    }
}
