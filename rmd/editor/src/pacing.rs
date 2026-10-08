use std::time::{Duration, Instant};

// long enough for ImGui hover delays, tooltips and popups to settle after the last activity
const SETTLE: Duration = Duration::from_millis(600);
const THROTTLED_INTERVAL: Duration = Duration::from_millis(33);
// matches the co-op cursor interval, so peer cursors stay smooth
const BUSY_INTERVAL: Duration = Duration::from_millis(50);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum FrameDemand {
    #[default]
    Idle,
    Throttled,
    Full,
}

impl FrameDemand {
    pub fn raise(&mut self, demand: Self) { *self = (*self).max(demand); }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NextFrame {
    Now,
    At(Instant),
    OnEvent,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Status {
    pub is_busy: bool,
    pub is_hidden: bool,
}

#[derive(Debug, Default)]
pub struct FramePacer {
    last_activity: Option<Instant>,
}

impl FramePacer {
    pub fn activity(&mut self, now: Instant) { self.last_activity = Some(now); }

    pub fn next(&self, now: Instant, frame_start: Instant, demand: FrameDemand, status: Status) -> NextFrame {
        // a hidden window presents nothing, so nothing would hold a frame back to the display rate
        let demand = if status.is_hidden { FrameDemand::Idle } else { demand };
        let is_settling = !status.is_hidden && self.last_activity.is_some_and(|at| now.duration_since(at) < SETTLE);
        if is_settling || demand == FrameDemand::Full {
            return NextFrame::Now;
        }

        let throttled = (demand == FrameDemand::Throttled).then(|| frame_start + THROTTLED_INTERVAL);
        let busy = status.is_busy.then(|| now + BUSY_INTERVAL);
        match throttled.into_iter().chain(busy).min() {
            Some(at) => NextFrame::At(at),
            None => NextFrame::OnEvent,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const IDLE: Status = Status {
        is_busy: false,
        is_hidden: false,
    };
    const BUSY: Status = Status {
        is_busy: true,
        is_hidden: false,
    };

    fn pacer_with_activity_at(at: Instant) -> FramePacer {
        let mut pacer = FramePacer::default();
        pacer.activity(at);

        pacer
    }

    #[test]
    fn activity_draws_at_full_rate_until_it_settles() {
        let start = Instant::now();
        let pacer = pacer_with_activity_at(start);

        assert_eq!(
            pacer.next(start + Duration::from_millis(100), start, FrameDemand::Idle, IDLE),
            NextFrame::Now
        );
        assert_eq!(
            pacer.next(start + SETTLE, start, FrameDemand::Idle, IDLE),
            NextFrame::OnEvent
        );
    }

    #[test]
    fn background_activity_restarts_settling_without_input() {
        let start = Instant::now();
        let mut pacer = pacer_with_activity_at(start);
        let completed = start + SETTLE * 2;

        assert_eq!(
            pacer.next(completed, completed, FrameDemand::Idle, BUSY),
            NextFrame::At(completed + BUSY_INTERVAL)
        );
        pacer.activity(completed);
        assert_eq!(
            pacer.next(completed, completed, FrameDemand::Idle, IDLE),
            NextFrame::Now
        );
        assert_eq!(
            pacer.next(
                completed + SETTLE - Duration::from_millis(1),
                completed,
                FrameDemand::Idle,
                IDLE
            ),
            NextFrame::Now
        );
        assert_eq!(
            pacer.next(completed + SETTLE, completed, FrameDemand::Idle, IDLE),
            NextFrame::OnEvent
        );
    }

    #[test]
    fn throttled_demand_draws_one_interval_after_its_frame_started() {
        let start = Instant::now();
        let now = start + Duration::from_millis(5);

        assert_eq!(
            FramePacer::default().next(now, start, FrameDemand::Throttled, IDLE),
            NextFrame::At(start + THROTTLED_INTERVAL)
        );
    }

    #[test]
    fn full_demand_draws_at_full_rate_without_input() {
        let start = Instant::now();

        assert_eq!(
            FramePacer::default().next(start, start, FrameDemand::Full, IDLE),
            NextFrame::Now
        );
    }

    #[test]
    fn background_work_polls_and_the_sooner_deadline_wins() {
        let start = Instant::now();

        assert_eq!(
            FramePacer::default().next(start, start, FrameDemand::Idle, BUSY),
            NextFrame::At(start + BUSY_INTERVAL)
        );
        assert_eq!(
            FramePacer::default().next(start, start, FrameDemand::Throttled, BUSY),
            NextFrame::At(start + THROTTLED_INTERVAL)
        );
    }

    #[test]
    fn nothing_to_do_waits_for_an_event() {
        let start = Instant::now();

        assert_eq!(
            FramePacer::default().next(start, start, FrameDemand::Idle, IDLE),
            NextFrame::OnEvent
        );
    }

    #[test]
    fn a_hidden_window_only_polls_background_work() {
        let start = Instant::now();
        let pacer = pacer_with_activity_at(start);
        let hidden = |is_busy| Status {
            is_busy,
            is_hidden: true,
        };

        assert_eq!(
            pacer.next(start, start, FrameDemand::Full, hidden(false)),
            NextFrame::OnEvent
        );
        assert_eq!(
            pacer.next(start, start, FrameDemand::Throttled, hidden(true)),
            NextFrame::At(start + BUSY_INTERVAL)
        );
    }

    #[test]
    fn demand_only_rises_within_a_frame() {
        let mut demand = FrameDemand::Full;
        demand.raise(FrameDemand::Throttled);
        assert_eq!(demand, FrameDemand::Full);

        demand = FrameDemand::Idle;
        demand.raise(FrameDemand::Throttled);
        assert_eq!(demand, FrameDemand::Throttled);
    }
}
