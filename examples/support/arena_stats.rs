//! Statistical helpers for the tournament example.

/// A two-sided 95% confidence interval.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Interval {
    pub low: f64,
    pub high: f64,
}

/// Cluster-level sufficient statistics for a ratio of sums.
///
/// A mirrored pair is one cluster.  For a decisive-win rate, `numerator`
/// is one bot's wins in the pair and `denominator` is the pair's decisive
/// rounds or games.  Keeping the cross moments lets pooled reports recover
/// a sandwich standard error without retaining every trial.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RatioMoments {
    pub clusters: u32,
    pub numerator: u64,
    pub denominator: u64,
    pub numerator_sq: u64,
    pub numerator_denominator: u64,
    pub denominator_sq: u64,
}

impl RatioMoments {
    pub fn record(&mut self, numerator: u32, denominator: u32) {
        let (numerator, denominator) = (u64::from(numerator), u64::from(denominator));
        self.clusters += 1;
        self.numerator += numerator;
        self.denominator += denominator;
        self.numerator_sq += numerator * numerator;
        self.numerator_denominator += numerator * denominator;
        self.denominator_sq += denominator * denominator;
    }

    pub fn merge(mut self, other: Self) -> Self {
        self.clusters += other.clusters;
        self.numerator += other.numerator;
        self.denominator += other.denominator;
        self.numerator_sq += other.numerator_sq;
        self.numerator_denominator += other.numerator_denominator;
        self.denominator_sq += other.denominator_sq;
        self
    }

    pub fn estimate(self) -> Option<f64> {
        (self.denominator != 0).then(|| self.numerator as f64 / self.denominator as f64)
    }

    /// A normal 95% interval with a pair-cluster sandwich standard error.
    pub fn cluster_interval(self) -> Option<Interval> {
        let interval = self.cluster_interval_unbounded()?;
        Some(Interval {
            low: interval.low.max(0.0),
            high: interval.high.min(1.0),
        })
    }

    /// An unbounded normal 95% interval for a clustered ratio or mean.
    pub fn cluster_interval_unbounded(self) -> Option<Interval> {
        let estimate = self.estimate()?;
        if self.clusters < 2 {
            return None;
        }

        let residual_ss = self.numerator_sq as f64
            - 2.0 * estimate * self.numerator_denominator as f64
            + estimate * estimate * self.denominator_sq as f64;
        let variance = f64::from(self.clusters) * residual_ss.max(0.0)
            / (f64::from(self.clusters - 1) * (self.denominator as f64).powi(2));
        let half = 1.96 * variance.sqrt();
        Some(Interval {
            low: estimate - half,
            high: estimate + half,
        })
    }
}

/// Cluster moments for a signed ratio, such as score margin per game.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SignedRatioMoments {
    pub clusters: u32,
    pub numerator: i64,
    pub denominator: u64,
    pub numerator_sq: u64,
    pub numerator_denominator: i64,
    pub denominator_sq: u64,
}

impl SignedRatioMoments {
    pub fn record(&mut self, numerator: i32, denominator: u32) {
        let numerator = i64::from(numerator);
        let denominator = u64::from(denominator);
        self.clusters += 1;
        self.numerator += numerator;
        self.denominator += denominator;
        self.numerator_sq += numerator.unsigned_abs() * numerator.unsigned_abs();
        self.numerator_denominator += numerator * denominator as i64;
        self.denominator_sq += denominator * denominator;
    }

    pub fn merge(mut self, other: Self) -> Self {
        self.clusters += other.clusters;
        self.numerator += other.numerator;
        self.denominator += other.denominator;
        self.numerator_sq += other.numerator_sq;
        self.numerator_denominator += other.numerator_denominator;
        self.denominator_sq += other.denominator_sq;
        self
    }

    pub fn estimate(self) -> Option<f64> {
        (self.denominator != 0).then(|| self.numerator as f64 / self.denominator as f64)
    }

    pub fn cluster_interval(self) -> Option<Interval> {
        let estimate = self.estimate()?;
        if self.clusters < 2 {
            return None;
        }
        let residual_ss = self.numerator_sq as f64
            - 2.0 * estimate * self.numerator_denominator as f64
            + estimate * estimate * self.denominator_sq as f64;
        let variance = f64::from(self.clusters) * residual_ss.max(0.0)
            / (f64::from(self.clusters - 1) * (self.denominator as f64).powi(2));
        let half = 1.96 * variance.sqrt();
        Some(Interval {
            low: estimate - half,
            high: estimate + half,
        })
    }
}

/// The 95% Wilson score interval for independent Bernoulli trials.
pub fn wilson(wins: u32, n: u32) -> Interval {
    if n == 0 {
        return Interval {
            low: 0.0,
            high: 1.0,
        };
    }
    let (wins, n) = (f64::from(wins), f64::from(n));
    let z = 1.96;
    let p = wins / n;
    let denom = 1.0 + z * z / n;
    let center = (p + z * z / (2.0 * n)) / denom;
    let half = z * (p * (1.0 - p) / n + z * z / (4.0 * n * n)).sqrt() / denom;
    Interval {
        low: center - half,
        high: center + half,
    }
}

/// The two-sided normal p-value of a z statistic.
pub fn normal_p_value(z: f64) -> f64 {
    // erfc via Abramowitz & Stegun 7.1.26, |error| < 1.5e-7.
    let x = z.abs() / std::f64::consts::SQRT_2;
    let t = 1.0 / (1.0 + 0.327_591_1 * x);
    let poly = t
        * (0.254_829_592
            + t * (-0.284_496_736
                + t * (1.421_413_741 + t * (-1.453_152_027 + t * 1.061_405_429))));
    poly * (-x * x).exp()
}

/// An exact p-value retained in log space.
///
/// Keeping the logarithm prevents publication-sized exact tests from being
/// silently rounded to the mathematically false value zero.  [`as_f64`](Self::as_f64)
/// returns `None` if the positive value is below the smallest `f64`, while
/// [`decimal`](Self::decimal) remains available at every supported sample size.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct ExactPValue {
    ln: f64,
}

impl ExactPValue {
    /// The null result used when a sign test has no non-tied observations.
    pub const fn one() -> Self {
        Self { ln: 0.0 }
    }

    /// Compute a two-sided exact sign-test p-value.
    ///
    /// Ties must be omitted before calling this function.  With no remaining
    /// observations there is no test, so this returns `None` rather than one;
    /// callers that need a total p-value for multiplicity correction can use
    /// [`one`](Self::one).
    pub fn from_signs(positive: u32, negative: u32) -> Option<Self> {
        let n = positive + negative;
        if n == 0 {
            return None;
        }
        let tail = positive.min(negative);
        let log_combination = (1..=tail).fold(0.0, |sum, i| {
            sum + f64::from(n - tail + i).ln() - f64::from(i).ln()
        });
        let log_largest = log_combination - f64::from(n) * std::f64::consts::LN_2;

        // Sum P(0)..P(tail) relative to P(tail), walking down by the exact
        // adjacent-term ratio P(k-1)/P(k) = k/(n-k+1).  Only the final
        // logarithm is retained, so the p-value itself never has to fit in an
        // `f64`.
        let mut relative_term = 1.0;
        let mut relative_sum = 1.0;
        for k in (1..=tail).rev() {
            relative_term *= f64::from(k) / f64::from(n - k + 1);
            relative_sum += relative_term;
        }
        let ln = (std::f64::consts::LN_2 + log_largest + relative_sum.ln()).min(0.0);
        Some(Self { ln })
    }

    /// The natural logarithm of the p-value.
    pub const fn ln(self) -> f64 {
        self.ln
    }

    /// Return the p-value as a positive `f64`, or `None` on underflow.
    pub fn as_f64(self) -> Option<f64> {
        let value = self.ln().exp();
        (value > 0.0).then_some(value)
    }

    /// A normalized scientific-decimal representation with 16 significant
    /// digits, including for values below the range of `f64`.
    pub fn decimal(self) -> String {
        let mut exponent = (self.ln() / std::f64::consts::LN_10).floor() as i64;
        let mantissa = (self.ln() - exponent as f64 * std::f64::consts::LN_10).exp();
        let mut digits = format!("{mantissa:.15}");

        // Formatting can round a value infinitesimally below ten up to ten.
        // Renormalize so consumers always receive one digit before the point.
        if digits.starts_with("10.") {
            exponent += 1;
            digits = "1.000000000000000".into();
        }
        format!("{digits}e{exponent:+}")
    }

    /// Multiply by a positive correction factor and clamp the result to one.
    // This shared support module is compiled independently into each example;
    // Holm-report examples use this method even though the arena binary does not.
    #[allow(dead_code)]
    pub fn multiply_clamped(self, factor: usize) -> Self {
        assert!(factor > 0, "a p-value correction factor must be positive");
        Self {
            ln: (self.ln + (factor as f64).ln()).min(0.0),
        }
    }
}

/// Exact two-sided sign-test p-value for `positive` versus `negative` signs.
///
/// This free function preserves the arena helpers' original entry point while
/// returning the underflow-safe representation.
pub fn exact_sign_p_value(positive: u32, negative: u32) -> Option<ExactPValue> {
    ExactPValue::from_signs(positive, negative)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ratio_moments_merge_and_estimate() {
        let mut left = RatioMoments::default();
        left.record(2, 2);
        left.record(1, 2);
        let mut right = RatioMoments::default();
        right.record(0, 2);
        right.record(1, 2);

        let all = left.merge(right);
        assert_eq!(all.clusters, 4);
        assert_eq!(all.estimate(), Some(0.5));
        let interval = all
            .cluster_interval()
            .expect("four clusters have an interval");
        assert!(interval.low < 0.5 && interval.high > 0.5);
    }

    #[test]
    fn ratio_interval_handles_variable_decisive_counts() {
        let mut moments = RatioMoments::default();
        moments.record(1, 1); // one win and one dead hand
        moments.record(0, 2);
        moments.record(1, 2);
        assert_eq!(moments.estimate(), Some(0.4));
        assert!(moments.cluster_interval().is_some());
    }

    #[test]
    fn signed_ratio_tracks_margin() {
        let mut moments = SignedRatioMoments::default();
        moments.record(20, 2);
        moments.record(-10, 2);
        assert_eq!(moments.estimate(), Some(2.5));
        let interval = moments
            .cluster_interval()
            .expect("two clusters have an interval");
        assert!(interval.low < 2.5 && interval.high > 2.5);
    }

    #[test]
    fn exact_sign_test_covers_ties_and_extremes() {
        assert_eq!(exact_sign_p_value(0, 0), None);
        assert_eq!(exact_sign_p_value(1, 1), Some(ExactPValue::one()));
        assert!((exact_sign_p_value(3, 0).unwrap().as_f64().unwrap() - 0.25).abs() < 1e-12);
        assert!((exact_sign_p_value(8, 2).unwrap().as_f64().unwrap() - 0.109_375).abs() < 1e-12);

        let extreme = exact_sign_p_value(4000, 0).expect("a unanimous test is defined");
        assert!(extreme.ln().is_finite());
        assert_eq!(extreme.as_f64(), None);
        assert!(extreme.decimal().starts_with("1.51721574069"));
        assert!(extreme.decimal().ends_with("e-1204"));
        assert_ne!(extreme.decimal(), "0");
    }

    #[test]
    fn exact_p_value_formats_and_corrects_without_underflow() {
        let ordinary = exact_sign_p_value(8, 2).expect("a sign test is defined");
        assert_eq!(ordinary.decimal(), "1.093750000000000e-1");
        assert_eq!(ExactPValue::one().decimal(), "1.000000000000000e+0");

        let extreme = exact_sign_p_value(4000, 0).unwrap();
        let corrected = extreme.multiply_clamped(6);
        assert!(corrected > extreme);
        assert_eq!(corrected.as_f64(), None);
        assert!(corrected.decimal().ends_with("e-1204"));
        assert_eq!(ExactPValue::one().multiply_clamped(6), ExactPValue::one());
    }

    #[test]
    fn unanimous_pair_clusters_have_zero_width_intervals() {
        let mut all_wins = RatioMoments::default();
        let mut all_losses = RatioMoments::default();
        for _ in 0..8 {
            all_wins.record(2, 2);
            all_losses.record(0, 2);
        }
        assert_eq!(
            all_wins.cluster_interval(),
            Some(Interval {
                low: 1.0,
                high: 1.0,
            })
        );
        assert_eq!(
            all_losses.cluster_interval(),
            Some(Interval {
                low: 0.0,
                high: 0.0,
            })
        );
    }

    #[test]
    fn wilson_contains_the_estimate() {
        let interval = wilson(40, 100);
        assert!(interval.low < 0.4 && interval.high > 0.4);
    }
}
