//! 图表与历史对比用的纯计算：直方图分箱、坐标轴刻度、两次测速的排名对比。

use crate::core::model::SpeedRecord;

/// 直方图的一个分箱 `[start, end)`。
#[derive(Clone, Debug, PartialEq)]
pub struct Bin {
    pub start: f64,
    pub end: f64,
    pub count: usize,
}

/// 「好看」的步长：1 / 2 / 2.5 / 5 × 10^k，使 `max / step` 不超过 `max_steps`。
pub fn nice_step(max: f64, max_steps: usize) -> f64 {
    if !max.is_finite() || max <= 0.0 || max_steps == 0 {
        return 1.0;
    }
    let raw = max / max_steps as f64;
    let magnitude = 10f64.powf(raw.log10().floor());
    for factor in [1.0, 2.0, 2.5, 5.0, 10.0] {
        let step = factor * magnitude;
        if step >= raw {
            return step;
        }
    }
    10.0 * magnitude
}

/// 坐标轴上限：不小于 `max` 的步长整数倍（至少一个步长）。
pub fn nice_ceiling(max: f64, max_steps: usize) -> (f64, f64) {
    let step = nice_step(max, max_steps);
    let ceiling = ((max / step).ceil() * step).max(step);
    (ceiling, step)
}

/// 从 0 到 `ceiling`（含）的刻度。
pub fn axis_ticks(ceiling: f64, step: f64) -> Vec<f64> {
    if step <= 0.0 || !ceiling.is_finite() {
        return vec![0.0];
    }
    let count = (ceiling / step).round() as usize;
    (0..=count).map(|index| index as f64 * step).collect()
}

/// 按好看的步长把数值分箱（从 0 开始，最多 `max_bins` 个）；忽略非正数与非有限值。
pub fn histogram(values: &[f64], max_bins: usize) -> Vec<Bin> {
    let values: Vec<f64> = values
        .iter()
        .copied()
        .filter(|value| value.is_finite() && *value > 0.0)
        .collect();
    let Some(max) = values.iter().copied().reduce(f64::max) else {
        return Vec::new();
    };
    let step = nice_step(max, max_bins.max(1));
    let bins = ((max / step).floor() as usize + 1).min(max_bins.max(1));
    let mut out: Vec<Bin> = (0..bins)
        .map(|index| Bin {
            start: index as f64 * step,
            end: (index + 1) as f64 * step,
            count: 0,
        })
        .collect();
    for value in values {
        let index = ((value / step).floor() as usize).min(bins - 1);
        out[index].count += 1;
    }
    out
}

/// 两次测速对比中的一行（以当前一次的 Top N 为准）。
#[derive(Clone, Debug, PartialEq)]
pub struct CompareRow {
    pub ip: String,
    pub colo: String,
    pub rank: usize,
    pub median: f64,
    /// 基准一次中的排名（1 起）；`None` 表示新上榜。
    pub prev_rank: Option<usize>,
    pub prev_median: Option<f64>,
}

impl CompareRow {
    /// 排名变化：正数为上升。
    pub fn rank_change(&self) -> Option<i64> {
        self.prev_rank.map(|prev| prev as i64 - self.rank as i64)
    }

    pub fn median_change(&self) -> Option<f64> {
        self.prev_median.map(|prev| self.median - prev)
    }
}

fn ranked(records: &[SpeedRecord]) -> Vec<&SpeedRecord> {
    let mut ranked: Vec<&SpeedRecord> = records.iter().collect();
    ranked.sort_by(|a, b| b.median.total_cmp(&a.median));
    ranked
}

/// 对比两次测速：返回当前 Top `top` 的每一行，以及基准 Top `top` 中掉出当前 Top 的数量。
/// 同一 IP 以 IP 地址判定（端口不同视为同一地址）。
pub fn compare_runs(
    current: &[SpeedRecord],
    base: &[SpeedRecord],
    top: usize,
) -> (Vec<CompareRow>, usize) {
    let current = ranked(current);
    let base = ranked(base);
    let rows: Vec<CompareRow> = current
        .iter()
        .take(top)
        .enumerate()
        .map(|(index, record)| {
            let previous = base.iter().position(|candidate| candidate.ip == record.ip);
            CompareRow {
                ip: record.ip.clone(),
                colo: record.colo.clone(),
                rank: index + 1,
                median: record.median,
                prev_rank: previous.map(|position| position + 1),
                prev_median: previous.map(|position| base[position].median),
            }
        })
        .collect();
    let dropped = base
        .iter()
        .take(top)
        .filter(|record| !rows.iter().any(|row| row.ip == record.ip))
        .count();
    (rows, dropped)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(ip: &str, median: f64) -> SpeedRecord {
        SpeedRecord {
            ip: ip.into(),
            port: 443,
            colo: "HKG".into(),
            median,
            best: median,
            min: median,
            rounds: vec![],
        }
    }

    #[test]
    fn nice_step_picks_round_numbers() {
        assert_eq!(nice_step(97.0, 10), 10.0);
        assert_eq!(nice_step(37.0, 8), 5.0);
        assert_eq!(nice_step(0.9, 10), 0.1);
        assert_eq!(nice_step(240.0, 10), 25.0);
        assert_eq!(nice_step(0.0, 10), 1.0);
    }

    #[test]
    fn ceiling_and_ticks_cover_the_max() {
        let (ceiling, step) = nice_ceiling(37.0, 4);
        assert_eq!((ceiling, step), (40.0, 10.0));
        assert_eq!(axis_ticks(ceiling, step), vec![0.0, 10.0, 20.0, 30.0, 40.0]);
    }

    #[test]
    fn histogram_bins_from_zero_and_counts_everything() {
        let bins = histogram(&[1.0, 4.0, 9.9, 10.0, 19.0, 0.0, f64::NAN], 10);
        assert_eq!(bins.len(), 10);
        assert_eq!(
            bins[0],
            Bin {
                start: 0.0,
                end: 2.0,
                count: 1
            }
        );
        assert_eq!(bins.iter().map(|bin| bin.count).sum::<usize>(), 5);
        assert_eq!(bins.last().unwrap().count, 1);
        assert!(histogram(&[], 10).is_empty());
    }

    #[test]
    fn compare_runs_tracks_rank_changes_and_dropouts() {
        let base = vec![record("a", 50.0), record("b", 40.0), record("c", 30.0)];
        let current = vec![record("b", 60.0), record("d", 55.0), record("a", 45.0)];
        let (rows, dropped) = compare_runs(&current, &base, 3);
        assert_eq!(rows[0].ip, "b");
        assert_eq!(rows[0].rank_change(), Some(1));
        assert_eq!(rows[0].median_change(), Some(20.0));
        assert_eq!(rows[1].ip, "d");
        assert_eq!(rows[1].prev_rank, None);
        assert_eq!(rows[2].rank_change(), Some(-2));
        assert_eq!(dropped, 1);
    }
}
