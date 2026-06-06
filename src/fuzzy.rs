//! Small reusable fuzzy ranking using the same nucleo configuration as the jump
//! palette (smart case and normalization). Lets pickers offer type-to-filter
//! without duplicating the full jump scoring pipeline. The matcher is cached in
//! a thread-local so its scratch buffers are reused across keystrokes and frames
//! instead of being reallocated on every call.

use std::cell::RefCell;

use nucleo_matcher::{Config, Matcher};

thread_local! {
    static MATCHER: RefCell<Matcher> = RefCell::new(Matcher::new(Config::DEFAULT));
}

/// Fuzzy-rank `candidates` against `query`, keeping only matches, best score
/// first. Each candidate carries its searchable haystacks (e.g. alias,
/// hostname, provider, tags); the best-scoring haystack decides the rank.
/// Ties keep input order. An empty query returns every candidate in input
/// order (no filtering).
pub fn rank<T>(query: &str, candidates: impl IntoIterator<Item = (T, Vec<String>)>) -> Vec<T> {
    let candidates = candidates.into_iter();
    if query.is_empty() {
        return candidates.map(|(item, _)| item).collect();
    }
    use nucleo_matcher::Utf32Str;
    use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};

    let pattern = Pattern::parse(query, CaseMatching::Smart, Normalization::Smart);
    MATCHER.with(|m| {
        let mut matcher = m.borrow_mut();
        let mut buf: Vec<char> = Vec::new();
        let mut scored: Vec<(T, u32)> = Vec::new();
        for (item, haystacks) in candidates {
            let mut best = 0u32;
            for h in &haystacks {
                buf.clear();
                if let Some(score) = pattern.score(Utf32Str::new(h, &mut buf), &mut matcher) {
                    best = best.max(score);
                }
            }
            if best > 0 {
                scored.push((item, best));
            }
        }
        // Stable sort by score descending; ties keep input (config) order.
        scored.sort_by_key(|(_, score)| std::cmp::Reverse(*score));
        scored.into_iter().map(|(item, _)| item).collect()
    })
}

/// Fuzzy-rank the candidate host indices by the standard host search fields
/// (alias, hostname, user, provider, and provider/user tags), best match first.
/// An empty query keeps the candidate order. Shared by every host picker
/// (tunnel, container, snippet) so type-to-filter behaves identically across the
/// app and searches the same fields everywhere.
pub fn rank_host_indices(
    hosts: &[crate::ssh_config::model::HostEntry],
    candidates: &[usize],
    query: &str,
) -> Vec<usize> {
    let scored: Vec<(usize, Vec<String>)> = candidates
        .iter()
        .filter_map(|&i| hosts.get(i).map(|h| (i, h)))
        .map(|(i, h)| {
            let mut haystacks = vec![h.alias.clone(), h.hostname.clone(), h.user.clone()];
            if let Some(p) = &h.provider {
                haystacks.push(p.clone());
            }
            haystacks.extend(h.tags.iter().cloned());
            haystacks.extend(h.provider_tags.iter().cloned());
            (i, haystacks)
        })
        .collect();
    rank(query, scored)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cand(items: &[(usize, &[&str])]) -> Vec<(usize, Vec<String>)> {
        items
            .iter()
            .map(|(i, hs)| (*i, hs.iter().map(|s| s.to_string()).collect()))
            .collect()
    }

    #[test]
    fn empty_query_returns_all_in_order() {
        let c = cand(&[(0, &["alpha"]), (1, &["beta"])]);
        assert_eq!(rank("", c), vec![0, 1]);
    }

    #[test]
    fn filters_out_non_matches() {
        let c = cand(&[
            (0, &["aws-api-eu"]),
            (1, &["db-primary"]),
            (2, &["aws-worker"]),
        ]);
        let got = rank("aws", c);
        assert!(got.contains(&0));
        assert!(got.contains(&2));
        assert!(!got.contains(&1), "db-primary must not match 'aws'");
    }

    #[test]
    fn subsequence_matches_fuzzily() {
        // 'awseu' should match 'aws-api-eu' as a subsequence.
        let c = cand(&[(0, &["aws-api-eu"]), (1, &["db-primary"])]);
        let got = rank("awseu", c);
        assert_eq!(got, vec![0]);
    }

    #[test]
    fn matches_any_haystack_field() {
        // Query hits the hostname haystack, not the alias.
        let c = cand(&[(0, &["bastion", "140.82.121.3"]), (1, &["db", "10.0.0.1"])]);
        let got = rank("140.82", c);
        assert_eq!(got, vec![0]);
    }

    #[test]
    fn equal_scores_keep_input_order() {
        // Two candidates with identical haystacks score equally; the stable
        // sort must preserve their input order.
        let c = cand(&[(0, &["aws"]), (1, &["aws"])]);
        assert_eq!(rank("aws", c), vec![0, 1]);
    }

    #[test]
    fn rank_host_indices_searches_all_fields_and_respects_candidates() {
        use crate::ssh_config::model::HostEntry;
        let hosts = vec![
            HostEntry {
                alias: "web1".into(),
                hostname: "10.0.0.1".into(),
                provider: Some("aws".into()),
                ..Default::default()
            },
            HostEntry {
                alias: "db".into(),
                hostname: "10.0.0.2".into(),
                tags: vec!["prod".into()],
                ..Default::default()
            },
            HostEntry {
                alias: "cache".into(),
                hostname: "10.0.0.3".into(),
                ..Default::default()
            },
        ];
        let all = [0usize, 1, 2];
        // Empty query keeps the candidate order.
        assert_eq!(rank_host_indices(&hosts, &all, ""), vec![0, 1, 2]);
        // Matches the provider field, not just alias/hostname.
        assert_eq!(rank_host_indices(&hosts, &all, "aws"), vec![0]);
        // Matches a tag.
        assert_eq!(rank_host_indices(&hosts, &all, "prod"), vec![1]);
        // Only ranks within the candidate set (index 0 excluded).
        assert_eq!(rank_host_indices(&hosts, &[1, 2], ""), vec![1, 2]);
    }
}
