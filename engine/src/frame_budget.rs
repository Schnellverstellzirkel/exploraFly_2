//! Exact, opt-in frame budget measurements. Successful queue-present calls are
//! submission cadence, not proof that a compositor displayed every image.
use std::time::{Duration, Instant};

pub const TARGET_FRAME_NS: u64 = 1_000_000;
const WARMUP_PRESENTS: u64 = 500;

#[derive(Debug, PartialEq)]
pub struct Distribution {
    pub samples: usize,
    pub mean_us: f64,
    pub p50_us: f64,
    pub p95_us: f64,
    pub p99_us: f64,
    pub max_us: f64,
    pub over_budget: usize,
}

impl Distribution {
    /// Nearest-rank percentiles; retain sub-microsecond precision and exact tails.
    pub fn from_ns(samples: &mut [u64]) -> Option<Self> {
        if samples.is_empty() {
            return None;
        }
        samples.sort_unstable();
        let rank = |percent: usize| samples[(samples.len() * percent).div_ceil(100) - 1];
        Some(Self {
            samples: samples.len(),
            mean_us: samples.iter().map(|&ns| ns as f64 / 1000.0).sum::<f64>()
                / samples.len() as f64,
            p50_us: rank(50) as f64 / 1000.0,
            p95_us: rank(95) as f64 / 1000.0,
            p99_us: rank(99) as f64 / 1000.0,
            max_us: *samples.last().unwrap() as f64 / 1000.0,
            over_budget: samples.iter().filter(|&&ns| ns > TARGET_FRAME_NS).count(),
        })
    }

    pub fn json(&self) -> String {
        format!(
            "{{\"samples\":{},\"mean_us\":{:.3},\"p50_us\":{:.3},\"p95_us\":{:.3},\"p99_us\":{:.3},\"max_us\":{:.3},\"over_1ms\":{}}}",
            self.samples, self.mean_us, self.p50_us, self.p95_us, self.p99_us,
            self.max_us, self.over_budget,
        )
    }
}

/// Timestamp counters may expose fewer than 64 bits and wrap between queries.
pub fn timestamp_delta(start: u64, end: u64, valid_bits: u32) -> u64 {
    match valid_bits {
        0 => 0,
        1..=63 => end.wrapping_sub(start) & ((1u64 << valid_bits) - 1),
        _ => end.wrapping_sub(start),
    }
}

pub struct Capture {
    target: u64,
    warmup: u64,
    last_present: Option<Instant>,
    pub submission_intervals_ns: Vec<u64>,
    pub gpu_ns: Vec<u64>,
    pub display_times_ns: Vec<u64>,
    pub first_present_id: u64,
    pub last_present_id: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Progress {
    Warmup,
    Started,
    Measuring,
    Complete,
}

impl Capture {
    pub fn new(target: u64) -> Self {
        assert!(target > 0, "--benchmark requires a positive present count");
        // Allocation occurs once at startup, never on the measured hot path.
        let capacity = usize::try_from(target).expect("benchmark count too large");
        Self {
            target,
            warmup: 0,
            last_present: None,
            submission_intervals_ns: Vec::with_capacity(capacity),
            gpu_ns: Vec::with_capacity(capacity),
            display_times_ns: Vec::with_capacity(capacity),
            first_present_id: 0,
            last_present_id: 0,
        }
    }

    pub fn restart(&mut self) {
        self.warmup = 0;
        self.last_present = None;
        self.submission_intervals_ns.clear();
        self.gpu_ns.clear();
        self.display_times_ns.clear();
        self.first_present_id = 0;
        self.last_present_id = 0;
    }

    pub fn submitted(&mut self, now: Instant, boot_elapsed: Duration, id: u64) -> Progress {
        if boot_elapsed < Duration::from_secs(2) {
            return Progress::Warmup;
        }
        if self.warmup < WARMUP_PRESENTS {
            self.warmup += 1;
            if self.warmup == WARMUP_PRESENTS {
                self.last_present = Some(now);
                self.first_present_id = id + 1;
                return Progress::Started;
            }
            return Progress::Warmup;
        }
        let previous = self.last_present.replace(now).unwrap();
        self.submission_intervals_ns.push(now.duration_since(previous).as_nanos() as u64);
        self.last_present_id = id;
        if self.submission_intervals_ns.len() as u64 >= self.target {
            Progress::Complete
        } else {
            Progress::Measuring
        }
    }

    pub fn gpu_sample(&mut self, submitted_id: u64, ns: u64) {
        // A reused swapchain slot returns older work: do not contaminate the
        // measured window with completed GPU work from the warmup interval.
        if self.first_present_id != 0 && submitted_id >= self.first_present_id {
            self.gpu_ns.push(ns);
        }
    }

    pub fn displayed(&mut self, submitted_id: u64, actual_time_ns: u64) {
        if self.first_present_id != 0
            && submitted_id >= self.first_present_id
            && submitted_id <= self.last_present_id
            && actual_time_ns != 0
        {
            self.display_times_ns.push(actual_time_ns);
        }
    }

    /// Print one JSON line suitable for collection by tools/benchmark-frames.py.
    pub fn report(&mut self, metadata: &str) -> String {
        let submissions = Distribution::from_ns(&mut self.submission_intervals_ns)
            .expect("benchmark finished without measurements");
        let gpu = Distribution::from_ns(&mut self.gpu_ns);
        self.display_times_ns.sort_unstable();
        self.display_times_ns.dedup();
        let displayed = self.display_times_ns.len();
        let mut intervals: Vec<u64> = self.display_times_ns.windows(2)
            .map(|p| p[1] - p[0]).collect();
        let display = Distribution::from_ns(&mut intervals);
        let submission_fps = if submissions.mean_us > 0.0 { 1_000_000.0 / submissions.mean_us } else { 0.0 };
        let submission_target = submission_fps >= 1000.0 && submissions.p99_us <= 1000.0;
        // Feedback is asynchronous and may cover only part of the capture. A
        // partial sample is informative, but cannot certify the display target.
        let display_target = if displayed == submissions.samples {
            display.as_ref().map(|d| (d.mean_us > 0.0 && d.mean_us <= 1000.0 && d.p99_us <= 1000.0).to_string())
                .unwrap_or_else(|| "null".into())
        } else {
            "null".into()
        };
        println!(
            "benchmark: present submissions {submission_fps:.1}/s | 1ms budget p50 {:.1} p95 {:.1} p99 {:.1} max {:.1} us | mean+p99 target {} | display feedback {displayed}/{}",
            submissions.p50_us, submissions.p95_us, submissions.p99_us,
            submissions.max_us, if submission_target { "PASS" } else { "FAIL" },
            submissions.samples,
        );
        format!(
            "{{\"schema\":\"explora.frame-budget.v1\",\"target_fps\":1000,\"target_frame_us\":1000,\"criterion\":\"mean_and_p99\",\"present_submissions_per_second\":{submission_fps:.3},\"submission_target_met\":{submission_target},\"display_target_met\":{display_target},\"display_feedback_frames\":{displayed},\"submission_interval\":{},\"gpu_frame\":{},\"display_interval\":{},\"config\":{metadata}}}",
            submissions.json(),
            gpu.map(|d| d.json()).unwrap_or_else(|| "null".into()),
            display.map(|d| d.json()).unwrap_or_else(|| "null".into()),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles_keep_hitches_and_sub_microsecond_precision() {
        let mut samples = (1..=100).map(|i| i * 10_000 + 500).collect::<Vec<_>>();
        samples.reverse();
        let d = Distribution::from_ns(&mut samples).unwrap();
        assert_eq!(d.p50_us, 500.5);
        assert_eq!(d.p95_us, 950.5);
        assert_eq!(d.p99_us, 990.5);
        assert_eq!(d.max_us, 1000.5);
        assert_eq!(d.over_budget, 1);
        assert!((d.mean_us - 505.5).abs() < 0.0001);
    }

    #[test]
    fn absent_samples_are_not_reported_as_zero_time() {
        assert!(Distribution::from_ns(&mut []).is_none());
        let d = Distribution::from_ns(&mut [TARGET_FRAME_NS]).unwrap();
        assert_eq!(d.over_budget, 0);
        assert_eq!(d.p99_us, 1000.0);
    }

    #[test]
    fn timestamp_wrap_uses_queue_valid_bits() {
        assert_eq!(timestamp_delta(250, 5, 8), 11);
        assert_eq!(timestamp_delta(u64::MAX - 4, 5, 64), 10);
        assert_eq!(timestamp_delta(10, 20, 0), 0);
    }

    #[test]
    fn rare_long_stalls_fail_average_budget_even_when_p99_passes() {
        let mut capture = Capture::new(100);
        capture.submission_intervals_ns = vec![500_000; 100];
        capture.submission_intervals_ns[99] = 100_000_000;
        assert!(capture.report("{}").contains("\"submission_target_met\":false"));
    }

    #[test]
    fn display_target_needs_complete_distinct_feedback() {
        let mut capture = Capture::new(2);
        capture.submission_intervals_ns = vec![500_000; 2];
        capture.display_times_ns = vec![1_000_000, 1_000_000];
        assert!(capture.report("{}").contains("\"display_target_met\":null"));
        capture.display_times_ns = vec![1_000_000, 2_000_000];
        assert!(capture.report("{}").contains("\"display_target_met\":true"));
        capture.display_times_ns = vec![1_000_000, 17_666_667];
        assert!(capture.report("{}").contains("\"display_target_met\":false"));
    }

    #[test]
    fn zero_duration_samples_do_not_emit_non_finite_json() {
        let mut capture = Capture::new(1);
        capture.submission_intervals_ns.push(0);
        let report = capture.report("{}");
        assert!(report.contains("\"present_submissions_per_second\":0.000"));
        assert!(report.contains("\"submission_target_met\":false"));
    }

    #[test]
    fn warmup_counts_presents_and_excludes_old_gpu_queries() {
        let start = Instant::now();
        let mut capture = Capture::new(2);
        assert_eq!(capture.submitted(start, Duration::ZERO, 1), Progress::Warmup);
        for id in 1..WARMUP_PRESENTS {
            assert_eq!(capture.submitted(start, Duration::from_secs(2), id), Progress::Warmup);
        }
        assert_eq!(capture.submitted(start, Duration::from_secs(2), 500), Progress::Started);
        capture.gpu_sample(500, 500_000);
        assert!(capture.gpu_ns.is_empty());
        assert_eq!(capture.submitted(start + Duration::from_micros(750), Duration::from_secs(2), 501), Progress::Measuring);
        assert_eq!(capture.submitted(start + Duration::from_micros(1750), Duration::from_secs(2), 502), Progress::Complete);
        assert_eq!(capture.submission_intervals_ns, [750_000, 1_000_000]);
        capture.gpu_sample(501, 500_000);
        capture.displayed(500, 900);
        capture.displayed(501, 1000);
        let report = capture.report("{}");
        assert!(report.contains("\"submission_target_met\":true"));
        assert!(report.contains("\"display_target_met\":null"));
        assert_eq!(capture.gpu_ns, [500_000]);
        capture.restart();
        assert!(capture.submission_intervals_ns.is_empty());
        assert_eq!(capture.first_present_id, 0);
    }
}
