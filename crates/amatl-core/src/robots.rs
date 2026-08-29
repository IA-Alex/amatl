//! `robots.txt` compliance for crawl-discovered URLs.
//!
//! AMATL makes two different kinds of request, and only one of them is
//! crawling:
//!
//! * A URL a user asked for — a Search result the operator chose to enrich — is
//!   a user agent acting on an explicit instruction. `robots.txt` governs
//!   automated discovery, not a fetch the human requested, so those are not
//!   gated here.
//! * A URL AMATL discovered by itself, by following a link at depth ≥ 1, is
//!   crawling. Those go through this module: the origin's `robots.txt` decides,
//!   and its `Crawl-delay` is honored within the Deep deadline.
//!
//! The parser implements the subset of RFC 9309 that matters for that
//! decision — user-agent groups, `Allow`/`Disallow` with longest-match wins and
//! `*`/`$` wildcards, plus the widely used non-standard `Crawl-delay`. It is
//! deliberately small and total: anything it cannot parse is ignored rather
//! than guessed at.
//!
//! Failure handling follows RFC 9309 §2.3.1: a 4xx (including 404, "no rules
//! published") allows the crawl, while a 5xx or an unreachable origin is
//! treated as full disallow so a broken origin is not hammered.

use crate::fetch::{FetchRequest, Fetcher};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use url::Url;

/// User-agent token AMATL matches itself against in a robots group.
pub const ROBOTS_USER_AGENT: &str = "amatl";
/// Upper bound for a `Crawl-delay` AMATL will actually wait, in milliseconds.
/// A larger declared delay means "skip this link", not "block Deep".
pub const MAXIMUM_CRAWL_DELAY_MS: u64 = 5_000;

/// What `robots.txt` says about one URL.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RobotsDecision {
    /// The crawl may proceed, after waiting the declared delay.
    Allowed { crawl_delay_ms: u64 },
    /// The origin disallows this path for our user agent.
    Disallowed,
    /// The declared `Crawl-delay` exceeds what Deep will wait.
    DelayTooLong,
    /// `robots.txt` could not be retrieved; the crawl fails closed.
    Unavailable,
}

impl RobotsDecision {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Allowed { .. } => "allowed",
            Self::Disallowed => "robots_disallowed",
            Self::DelayTooLong => "robots_crawl_delay_too_long",
            Self::Unavailable => "robots_unavailable",
        }
    }
}

/// Rules parsed from one `robots.txt`, already narrowed to our user agent.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RobotsRules {
    /// `(path pattern, allowed)` in declaration order.
    rules: Vec<(String, bool)>,
    crawl_delay_ms: Option<u64>,
    /// No group applied to us and no global group exists.
    empty: bool,
}

impl RobotsRules {
    /// Rules that allow everything, used for origins that publish none.
    pub fn allow_all() -> Self {
        Self {
            rules: vec![],
            crawl_delay_ms: None,
            empty: true,
        }
    }

    /// Parse `robots.txt`, keeping the most specific group that applies to
    /// `user_agent`: an exact token match wins over the `*` group.
    pub fn parse(body: &str, user_agent: &str) -> Self {
        let mut specific = Group::default();
        let mut wildcard = Group::default();
        // Consecutive `User-agent` lines share the group that follows them.
        let mut current: Vec<String> = vec![];
        let mut collecting_agents = false;
        for line in body.lines() {
            let line = line.split('#').next().unwrap_or_default().trim();
            if line.is_empty() {
                continue;
            }
            let Some((field, value)) = line.split_once(':') else {
                continue;
            };
            let field = field.trim().to_ascii_lowercase();
            let value = value.trim();
            match field.as_str() {
                "user-agent" => {
                    if !collecting_agents {
                        current.clear();
                        collecting_agents = true;
                    }
                    current.push(value.to_ascii_lowercase());
                }
                "allow" | "disallow" | "crawl-delay" => {
                    collecting_agents = false;
                    for agent in &current {
                        let group = if agent == &user_agent.to_ascii_lowercase() {
                            &mut specific
                        } else if agent == "*" {
                            &mut wildcard
                        } else {
                            continue;
                        };
                        match field.as_str() {
                            "allow" if !value.is_empty() => {
                                group.rules.push((value.to_owned(), true))
                            }
                            // An empty `Disallow` means "allow everything".
                            "disallow" if value.is_empty() => group.touched = true,
                            "disallow" => group.rules.push((value.to_owned(), false)),
                            "crawl-delay" => {
                                group.crawl_delay_ms = parse_delay_ms(value);
                            }
                            _ => {}
                        }
                        group.touched = true;
                    }
                }
                _ => {
                    collecting_agents = false;
                }
            }
        }
        let group = if specific.touched { specific } else { wildcard };
        Self {
            empty: !group.touched,
            rules: group.rules,
            crawl_delay_ms: group.crawl_delay_ms,
        }
    }

    /// Decide one path, applying longest-match-wins as RFC 9309 requires.
    pub fn decide(&self, path_and_query: &str) -> RobotsDecision {
        let delay = self.crawl_delay_ms.unwrap_or(0);
        if delay > MAXIMUM_CRAWL_DELAY_MS {
            return RobotsDecision::DelayTooLong;
        }
        if self.empty {
            return RobotsDecision::Allowed {
                crawl_delay_ms: delay,
            };
        }
        let mut best: Option<(usize, bool)> = None;
        for (pattern, allowed) in &self.rules {
            if let Some(length) = match_length(pattern, path_and_query) {
                // Longest match wins; `Allow` wins a tie, per the RFC.
                let replace = match best {
                    None => true,
                    Some((best_length, best_allowed)) => {
                        length > best_length || (length == best_length && *allowed && !best_allowed)
                    }
                };
                if replace {
                    best = Some((length, *allowed));
                }
            }
        }
        match best {
            Some((_, false)) => RobotsDecision::Disallowed,
            _ => RobotsDecision::Allowed {
                crawl_delay_ms: delay,
            },
        }
    }
}

#[derive(Default)]
struct Group {
    rules: Vec<(String, bool)>,
    crawl_delay_ms: Option<u64>,
    touched: bool,
}

/// Length of `pattern` when it matches `path`, or `None`.
///
/// Supports the two wildcards in common use: `*` for any sequence and a
/// trailing `$` anchoring the end of the path.
fn match_length(pattern: &str, path: &str) -> Option<usize> {
    let anchored = pattern.ends_with('$');
    let pattern = pattern.strip_suffix('$').unwrap_or(pattern);
    let mut cursor = 0_usize;
    let mut segments = pattern.split('*');
    let first = segments.next()?;
    if !path.starts_with(first) {
        return None;
    }
    cursor += first.len();
    let mut last_end = cursor;
    for segment in segments {
        if segment.is_empty() {
            last_end = cursor;
            continue;
        }
        let found = path.get(cursor..)?.find(segment)? + cursor;
        cursor = found + segment.len();
        last_end = cursor;
    }
    if anchored && last_end != path.len() {
        return None;
    }
    Some(pattern.len())
}

fn parse_delay_ms(value: &str) -> Option<u64> {
    let seconds = value.parse::<f64>().ok()?;
    if !seconds.is_finite() || seconds < 0.0 {
        return None;
    }
    Some((seconds * 1_000.0).round() as u64)
}

/// Per-origin `robots.txt` cache shared by one Deep run.
///
/// Bounded in entries and refreshed by TTL: a crawl must not turn into a second
/// crawl of `robots.txt` itself.
#[derive(Clone)]
pub struct RobotsCache {
    fetcher: Arc<dyn Fetcher>,
    entries: Arc<Mutex<BTreeMap<String, CacheEntry>>>,
    timeout_ms: u64,
    max_bytes: u64,
    ttl_seconds: i64,
    max_entries: usize,
}

#[derive(Clone)]
struct CacheEntry {
    rules: Option<RobotsRules>,
    fetched_at: i64,
}

impl RobotsCache {
    pub fn new(fetcher: Arc<dyn Fetcher>, timeout_ms: u64, max_bytes: u64) -> Self {
        Self {
            fetcher,
            entries: Arc::new(Mutex::new(BTreeMap::new())),
            timeout_ms,
            max_bytes,
            ttl_seconds: 3_600,
            max_entries: 64,
        }
    }

    /// Decide whether a discovered URL may be crawled.
    pub async fn decide(&self, url: &Url, request_id: Option<String>) -> RobotsDecision {
        let Some(origin) = origin_key(url) else {
            return RobotsDecision::Unavailable;
        };
        let now = crate::telemetry::now_unix();
        if let Some(entry) = self.cached(&origin, now) {
            return match entry {
                Some(rules) => rules.decide(&path_and_query(url)),
                None => RobotsDecision::Unavailable,
            };
        }
        let rules = self.load(&origin, request_id).await;
        self.store(origin, rules.clone(), now);
        match rules {
            Some(rules) => rules.decide(&path_and_query(url)),
            None => RobotsDecision::Unavailable,
        }
    }

    fn cached(&self, origin: &str, now: i64) -> Option<Option<RobotsRules>> {
        let entries = self.entries.lock().ok()?;
        let entry = entries.get(origin)?;
        (now - entry.fetched_at <= self.ttl_seconds).then(|| entry.rules.clone())
    }

    fn store(&self, origin: String, rules: Option<RobotsRules>, now: i64) {
        let Ok(mut entries) = self.entries.lock() else {
            return;
        };
        if entries.len() >= self.max_entries {
            entries.clear();
        }
        entries.insert(
            origin,
            CacheEntry {
                rules,
                fetched_at: now,
            },
        );
    }

    /// `None` means "could not be retrieved", which callers treat as disallow.
    async fn load(&self, origin: &str, request_id: Option<String>) -> Option<RobotsRules> {
        let url = Url::parse(&format!("{origin}/robots.txt")).ok()?;
        let result = self
            .fetcher
            .fetch(FetchRequest {
                url,
                timeout_ms: self.timeout_ms,
                max_bytes: self.max_bytes,
                // A robots.txt behind redirects is an edge case, not a
                // requirement; one hop is enough and keeps the cost bounded.
                max_redirects: 1,
                headers: BTreeMap::from([
                    ("accept".into(), "text/plain".into()),
                    (
                        "user-agent".into(),
                        format!("{ROBOTS_USER_AGENT}/0.1 (+safe-deep-fetch)"),
                    ),
                ]),
                request_id,
            })
            .await;
        match result {
            // No rules published: RFC 9309 says the crawl is allowed.
            Ok(response) if (400..500).contains(&response.status) => Some(RobotsRules::allow_all()),
            Ok(response) if response.status == 200 => Some(RobotsRules::parse(
                &String::from_utf8_lossy(&response.body),
                ROBOTS_USER_AGENT,
            )),
            // 5xx or unreachable: fail closed rather than assume consent.
            _ => None,
        }
    }
}

fn origin_key(url: &Url) -> Option<String> {
    let host = url.host_str()?;
    let scheme = url.scheme();
    Some(match url.port() {
        Some(port) => format!("{scheme}://{host}:{port}"),
        None => format!("{scheme}://{host}"),
    })
}

fn path_and_query(url: &Url) -> String {
    match url.query() {
        Some(query) => format!("{}?{query}", url.path()),
        None => url.path().to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
User-agent: *
Disallow: /private/
Crawl-delay: 1

User-agent: amatl
Disallow: /internal/
Allow: /internal/public/
Disallow: /*.pdf$
";

    fn rules() -> RobotsRules {
        RobotsRules::parse(SAMPLE, ROBOTS_USER_AGENT)
    }

    #[test]
    fn the_group_for_our_agent_wins_over_the_wildcard_group() {
        let rules = rules();
        // `/private/` belongs to the wildcard group, which no longer applies.
        assert_eq!(
            rules.decide("/private/x"),
            RobotsDecision::Allowed { crawl_delay_ms: 0 }
        );
        assert_eq!(rules.decide("/internal/x"), RobotsDecision::Disallowed);
    }

    #[test]
    fn longest_match_wins_and_allow_breaks_the_tie() {
        let rules = rules();
        assert_eq!(
            rules.decide("/internal/public/report"),
            RobotsDecision::Allowed { crawl_delay_ms: 0 }
        );
        let tie = RobotsRules::parse(
            "User-agent: *\nDisallow: /docs\nAllow: /docs\n",
            ROBOTS_USER_AGENT,
        );
        assert_eq!(
            tie.decide("/docs/a"),
            RobotsDecision::Allowed { crawl_delay_ms: 0 }
        );
    }

    #[test]
    fn wildcards_and_end_anchors_are_honored() {
        let rules = rules();
        assert_eq!(rules.decide("/reports/a.pdf"), RobotsDecision::Disallowed);
        assert_eq!(
            rules.decide("/reports/a.pdf.html"),
            RobotsDecision::Allowed { crawl_delay_ms: 0 }
        );
    }

    #[test]
    fn wildcard_group_applies_when_no_specific_group_exists() {
        let rules = RobotsRules::parse(
            "User-agent: *\nDisallow: /private/\nCrawl-delay: 0.5\n",
            ROBOTS_USER_AGENT,
        );
        assert_eq!(rules.decide("/private/x"), RobotsDecision::Disallowed);
        assert_eq!(
            rules.decide("/public/x"),
            RobotsDecision::Allowed {
                crawl_delay_ms: 500
            }
        );
    }

    #[test]
    fn an_empty_disallow_allows_everything() {
        let rules = RobotsRules::parse("User-agent: *\nDisallow:\n", ROBOTS_USER_AGENT);
        assert_eq!(
            rules.decide("/anything"),
            RobotsDecision::Allowed { crawl_delay_ms: 0 }
        );
    }

    #[test]
    fn a_delay_longer_than_deep_will_wait_stops_the_crawl() {
        let rules = RobotsRules::parse(
            "User-agent: *\nCrawl-delay: 120\nDisallow: /x\n",
            ROBOTS_USER_AGENT,
        );
        assert_eq!(rules.decide("/anything"), RobotsDecision::DelayTooLong);
    }

    #[test]
    fn unparseable_lines_are_ignored_rather_than_guessed() {
        let rules = RobotsRules::parse(
            "garbage\nUser-agent: *\n# comment\nDisallow: /a # trailing\nNonsense\n",
            ROBOTS_USER_AGENT,
        );
        assert_eq!(rules.decide("/a"), RobotsDecision::Disallowed);
        assert_eq!(
            rules.decide("/b"),
            RobotsDecision::Allowed { crawl_delay_ms: 0 }
        );
    }
}
