use ubu_core::UbuTimestamp;

use crate::errors::{AppError, Result};

/// Format a schedule coordinate without adding a time dependency or changing core.
pub fn timestamp_at(seconds: u64) -> Result<String> {
    let invalid = || AppError::bad_request_diagnostic(
        "invalid_schedule_timestamp", "schedule coordinate is outside RFC 3339 UTC range",
    );
    let epoch = UbuTimestamp::parse("1970-01-01T00:00:00Z")?.inner();
    let duration = std::time::Duration::from_secs(seconds).try_into().map_err(|_| invalid())?;
    let timestamp = epoch.checked_add(duration).filter(|time| time.year() <= 9999).ok_or_else(invalid)?;
    Ok(format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        timestamp.year(), u8::from(timestamp.month()), timestamp.day(),
        timestamp.hour(), timestamp.minute(), timestamp.second()))
}

/// Clock for planning horizons, next-action selection, and recorded execution actions.
/// Tests and replays can drive both planning and observed action timestamps.
pub trait PlanningClock: Send + Sync {
    fn now(&self) -> UbuTimestamp;
}

pub struct SystemClock;
impl PlanningClock for SystemClock {
    fn now(&self) -> UbuTimestamp { UbuTimestamp::now_utc() }
}

pub struct FixedClock(pub UbuTimestamp);
impl PlanningClock for FixedClock {
    fn now(&self) -> UbuTimestamp { self.0 }
}
