use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

use crate::core::model::SegmentScore;

pub const DEFAULT_STRATEGY: &str = "balanced";
/// 通用推荐起手段：AS906 内长期稳定、跨网络命中率高。
pub const SEED_SEGMENT: &str = "64.186.224.0/24";
pub const STRATEGIES: [&str; 3] = ["balanced", "sequential", "network"];

pub fn normalize_strategy(strategy: &str) -> &'static str {
    match strategy.trim().to_ascii_lowercase().as_str() {
        "sequential" => "sequential",
        "network" => "network",
        _ => DEFAULT_STRATEGY,
    }
}

pub fn strategy_label(strategy: &str) -> &'static str {
    match normalize_strategy(strategy) {
        "sequential" => "顺序覆盖",
        "network" => "网络优选",
        _ => "均衡轮转",
    }
}

pub fn strategy_detail(strategy: &str) -> &'static str {
    match normalize_strategy(strategy) {
        "sequential" => "按段池原始顺序逐段推进，适合完整覆盖所有段",
        "network" => "按本机粗测评分从高到低，未测段按均衡顺序补齐",
        _ => "跨 /16 轮转取样，起步固定为通用推荐段 64.186.224.0/24",
    }
}

pub fn group_key(segment: &str) -> String {
    let base = segment.split('/').next().unwrap_or(segment);
    let mut parts = base.split('.');
    match (parts.next(), parts.next()) {
        (Some(a), Some(b)) if !a.is_empty() && !b.is_empty() => format!("{a}.{b}"),
        _ => "?".to_string(),
    }
}

fn network_of(segment: &str) -> u32 {
    segment
        .split('/')
        .next()
        .and_then(|base| base.parse::<std::net::Ipv4Addr>().ok())
        .map(u32::from)
        .unwrap_or(u32::MAX)
}

fn dedupe(pool: &[String]) -> Vec<String> {
    let mut out = Vec::with_capacity(pool.len());
    let mut seen = HashSet::with_capacity(pool.len());
    for segment in pool {
        let segment = segment.trim();
        if segment.is_empty() {
            continue;
        }
        if seen.insert(segment.to_string()) {
            out.push(segment.to_string());
        }
    }
    out
}

pub fn ordered_pool(pool: &[String], strategy: &str, scores: &[SegmentScore]) -> Vec<String> {
    let unique = dedupe(pool);
    if unique.is_empty() {
        return unique;
    }
    match normalize_strategy(strategy) {
        "sequential" => unique,
        "network" => network_order(&unique, scores).unwrap_or_else(|| balanced_order(&unique)),
        _ => balanced_order(&unique),
    }
}

/// 均衡：先放通用推荐段，再按 /16 分组轮转，让每一批都覆盖尽量多的网络。
fn balanced_order(pool: &[String]) -> Vec<String> {
    let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for segment in pool {
        groups
            .entry(group_key(segment))
            .or_default()
            .push(segment.clone());
    }

    let seed_group = group_key(SEED_SEGMENT);
    let mut seed_bucket = groups.remove(&seed_group);
    if let Some(list) = seed_bucket.as_mut() {
        list.sort_by_key(|segment| network_of(segment));
    }

    let mut other_groups: Vec<(String, Vec<String>)> = groups.into_iter().collect();
    for (_, list) in other_groups.iter_mut() {
        list.sort_by_key(|segment| network_of(segment));
    }
    other_groups.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then_with(|| a.0.cmp(&b.0)));

    let mut out = Vec::with_capacity(pool.len());
    if let Some(list) = seed_bucket.as_mut()
        && let Some(index) = list.iter().position(|segment| segment == SEED_SEGMENT)
    {
        out.push(list.remove(index));
    }

    let mut buckets: Vec<VecDeque<String>> = other_groups
        .into_iter()
        .map(|(_, list)| VecDeque::from(list))
        .collect();
    // 推荐段所在 /16 的其余段排到轮转末尾，避免开头被同一组占满。
    if let Some(list) = seed_bucket
        && !list.is_empty()
    {
        buckets.push(VecDeque::from(list));
    }

    while out.len() < pool.len() {
        let mut moved = false;
        for bucket in buckets.iter_mut() {
            if let Some(segment) = bucket.pop_front() {
                out.push(segment);
                moved = true;
            }
        }
        if !moved {
            break;
        }
    }
    out
}

/// 网络：所有粗测有效的段优先（按得分排序），其余段按均衡顺序补齐。
fn network_order(pool: &[String], scores: &[SegmentScore]) -> Option<Vec<String>> {
    let mut by_segment: HashMap<&str, f64> = HashMap::new();
    for record in scores.iter().filter(|record| record.score > 0.0) {
        by_segment.insert(record.segment.as_str(), record.score);
    }
    if by_segment.is_empty() {
        return None;
    }
    let score = |segment: &str| by_segment.get(segment).copied().unwrap_or(0.0);

    let mut measured: Vec<&String> = pool.iter().filter(|segment| score(segment) > 0.0).collect();
    if measured.is_empty() {
        return None;
    }
    measured.sort_by(|a, b| {
        score(b)
            .total_cmp(&score(a))
            .then_with(|| network_of(a).cmp(&network_of(b)))
    });

    let unmeasured: Vec<String> = pool
        .iter()
        .filter(|segment| score(segment) <= 0.0)
        .cloned()
        .collect();
    let mut out: Vec<String> = measured.into_iter().cloned().collect();
    out.extend(balanced_order(&unmeasured));
    Some(out)
}

pub fn batch_size(batch: usize, len: usize) -> usize {
    if len == 0 { 0 } else { batch.max(1).min(len) }
}

pub fn normalize_cursor(cursor: usize, len: usize) -> usize {
    if len == 0 { 0 } else { cursor % len }
}

pub fn batch_segments(ordered: &[String], cursor: usize, batch: usize) -> Vec<String> {
    let len = ordered.len();
    let batch = batch_size(batch, len);
    if batch == 0 {
        return Vec::new();
    }
    let cursor = normalize_cursor(cursor, len);
    (0..batch)
        .map(|offset| ordered[(cursor + offset) % len].clone())
        .collect()
}

/// 返回 (下一游标, 是否跨轮)。
pub fn advance(cursor: usize, batch: usize, len: usize) -> (usize, bool) {
    if len == 0 {
        return (0, false);
    }
    let cursor = normalize_cursor(cursor, len);
    let batch = batch_size(batch, len);
    let wrapped = cursor + batch >= len;
    ((cursor + batch) % len, wrapped)
}

pub fn batch_positions(cursor: usize, batch: usize, len: usize) -> String {
    if len == 0 {
        return "—".to_string();
    }
    let batch = batch_size(batch, len);
    let cursor = normalize_cursor(cursor, len);
    if batch == len {
        return format!("全部 {len} 段");
    }
    if cursor + batch <= len {
        format!("第 {}–{} 段", cursor + 1, cursor + batch)
    } else {
        let tail = len - cursor;
        format!("第 {}–{} 段 + 第 1–{} 段", cursor + 1, len, batch - tail)
    }
}

pub fn index_of(ordered: &[String], segment: &str) -> Option<usize> {
    ordered.iter().position(|item| item == segment)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_pool() -> Vec<String> {
        [
            "1.1.1.0/24",
            "1.1.2.0/24",
            "2.2.2.0/24",
            "3.3.1.0/24",
            "64.186.224.0/24",
            "64.186.225.0/24",
            "64.186.240.0/24",
        ]
        .iter()
        .map(|item| item.to_string())
        .collect()
    }

    #[test]
    fn balanced_starts_with_recommended_seed_and_covers_all() {
        let pool = sample_pool();
        let ordered = ordered_pool(&pool, "balanced", &[]);
        assert_eq!(ordered.len(), pool.len());
        assert_eq!(ordered[0], SEED_SEGMENT);
        let unique: HashSet<_> = ordered.iter().collect();
        assert_eq!(unique.len(), pool.len());
    }

    #[test]
    fn balanced_interleaves_groups() {
        let ordered = ordered_pool(&sample_pool(), "balanced", &[]);
        assert_eq!(
            ordered,
            vec![
                "64.186.224.0/24",
                "1.1.1.0/24",
                "2.2.2.0/24",
                "3.3.1.0/24",
                "64.186.225.0/24",
                "1.1.2.0/24",
                "64.186.240.0/24",
            ]
        );
    }

    #[test]
    fn sequential_keeps_original_order() {
        let ordered = ordered_pool(&sample_pool(), "sequential", &[]);
        assert_eq!(ordered[0], "1.1.1.0/24");
        assert_eq!(ordered[4], SEED_SEGMENT);
    }

    #[test]
    fn network_orders_by_score_and_falls_back() {
        let pool = sample_pool();
        let scores = vec![SegmentScore {
            segment: "3.3.1.0/24".into(),
            group: "3.3".into(),
            samples: 2,
            hits: 2,
            colo: "HKG".into(),
            best_latency_ms: 30.0,
            avg_latency_ms: 35.0,
            mbps: Some(80.0),
            score: 1080.0,
        }];
        let ordered = ordered_pool(&pool, "network", &scores);
        assert_eq!(ordered[0], "3.3.1.0/24");
        assert_eq!(ordered[1], SEED_SEGMENT);
        let empty = ordered_pool(&pool, "network", &[]);
        assert_eq!(empty[0], SEED_SEGMENT);
    }

    #[test]
    fn batch_wraps_and_reports_positions() {
        let ordered = sample_pool();
        let batch = batch_segments(&ordered, 6, 3);
        assert_eq!(batch, vec!["64.186.240.0/24", "1.1.1.0/24", "1.1.2.0/24"]);
        assert_eq!(batch_positions(6, 3, 7), "第 7–7 段 + 第 1–2 段");
        assert_eq!(advance(6, 3, 7), (2, true));
        assert_eq!(advance(2, 3, 7), (5, false));
        assert_eq!(batch_positions(2, 3, 7), "第 3–5 段");
    }

    #[test]
    fn cursor_and_batch_are_clamped() {
        let ordered = sample_pool();
        assert_eq!(normalize_cursor(9, 7), 2);
        assert_eq!(batch_size(99, 7), 7);
        assert_eq!(batch_segments(&ordered, 2, 99).len(), 7);
        assert_eq!(batch_size(0, 0), 0);
    }
}
