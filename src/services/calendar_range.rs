//! The same exclusive overlap bounds used by Calendar's timeMin/timeMax.
use super::calendar_projection::DesiredEvent;
use ubu_core::UbuTimestamp;

#[derive(Debug, Clone)]
pub struct CalendarTimeRange {
    pub start: UbuTimestamp,
    pub end: UbuTimestamp,
}

impl CalendarTimeRange {
    pub fn parse(start: &str, end: &str) -> Result<Self, String> {
        let start = UbuTimestamp::parse(start).map_err(|_| "invalid range start")?;
        let end = UbuTimestamp::parse(end).map_err(|_| "invalid range end")?;
        if start >= end {
            return Err("range start must precede end".into());
        }
        Ok(Self { start, end })
    }

    pub fn overlaps(&self, event: &DesiredEvent) -> bool {
        match (
            UbuTimestamp::parse(&event.start_at),
            UbuTimestamp::parse(&event.end_at),
        ) {
            (Ok(start), Ok(end)) => start < self.end && end > self.start,
            _ => false,
        }
    }

    pub async fn planning(state: &crate::state::AppState) -> crate::errors::Result<Self> {
        let now = u64::try_from(state.planning_now().inner().unix_timestamp()).map_err(|_| {
            crate::errors::AppError::Internal("planning clock precedes epoch".into())
        })?;
        let range = super::planning_service::resolve_time_window(state, None, now).await?;
        Self::parse(
            &crate::planning_time::timestamp_at(range.start)?,
            &crate::planning_time::timestamp_at(range.end)?,
        )
        .map_err(crate::errors::AppError::Internal)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::calendar_client::{CalendarApi, RecordingCalendarApi};
    #[tokio::test]
    async fn recording_bounds_match_exclusive_google_overlap_and_offsets() {
        let range =
            CalendarTimeRange::parse("2026-09-25T09:00:00Z", "2026-09-25T10:00:00Z").unwrap();
        let events: Vec<_> = [
            ("aaaaa", "08:00:00Z", "09:00:00Z"),
            ("bbbbb", "08:30:00Z", "09:30:00Z"),
            ("ccccc", "10:00:00Z", "11:00:00Z"),
            ("ddddd", "11:15:00+02:00", "11:30:00+02:00"),
        ]
        .into_iter()
        .map(|(id, start, end)| DesiredEvent {
            external_id: id.into(),
            task_id: format!("task_{id}"),
            summary: "Synthetic".into(),
            start_at: format!("2026-09-25T{start}"),
            end_at: format!("2026-09-25T{end}"),
            color_id: None,
            transparent: false,
            reminders_minutes: vec![],
        })
        .collect();
        let client = RecordingCalendarApi::with_events(events);
        let selected = client.list_events(&range).await.unwrap();
        assert_eq!(
            selected
                .iter()
                .map(|e| e.external_id.as_str())
                .collect::<Vec<_>>(),
            ["bbbbb", "ddddd"]
        );
        assert!(CalendarTimeRange::parse("bad", "bad").is_err());
        assert!(CalendarTimeRange::parse("2026-09-25T10:00:00Z", "2026-09-25T09:00:00Z").is_err());
    }
}
