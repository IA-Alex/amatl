//! A small "Optics" DSL for operator-declared ranking overrides.
//!
//! # What this is
//!
//! An operator can drop a static `.optic` file next to `amatl.toml` and point
//! `[ranking.optics]` at it. The file declares `Boost` / `Downrank` rules that
//! match on a result's site, domain, URL or title, plus an optional
//! `DiscardNonMatching` directive that drops every result no rule matched.
//! [`ranking_adjustments::apply_adjustments`](crate::ranking_adjustments) folds
//! the parsed document in alongside the tracker penalty, before its final
//! re-sort.
//!
//! # Relationship to Stract
//!
//! The *grammar shape* is inspired by the syntax of Stract's public
//! `sample-optics` (`Rule { Matches { Site("|...|") }, Action(Boost(n)) }`,
//! `DiscardNonMatching`, OR across several `Matches` blocks within one `Rule`).
//! None of Stract's code is vendored here — Stract is AGPL-3.0 and this is an
//! independent parser and interpreter. Only four match fields are supported
//! (`Site`, `Domain`, `Title`, `Url`); `Description` / `Content` / `Schema`
//! and per-rule `Action(Discard)` are deliberately out of scope.
//!
//! # Scoring scale
//!
//! `Boost(n)` / `Downrank(n)` translate to a multiplier on the post-`rank()`
//! score. **This scale is an AMATL decision, not Stract's internal scale**
//! (which was not investigated and does not need to be replicated — only the
//! DSL surface syntax was verified against real `.optic` files):
//!
//! - `Boost(n)`   → `score *= 1.0 + n * 0.1`, then clamp to `1.0`.
//! - `Downrank(n)` → `score *= (1.0 - n * 0.1).max(0.01)`.
//!
//! Same mechanism as
//! [`TRACKER_PENALTY_MULTIPLIER`](crate::ranking_adjustments): a multiplier,
//! never a term in the protected `RankingPolicyV1` weighted sum.

use crate::model::RankedResult;
use nom::branch::alt;
use nom::bytes::complete::{tag, take_until, take_while1};
use nom::character::complete::{char, digit1, multispace1};
use nom::combinator::{map, map_res, opt, recognize, value};
use nom::multi::{many0, many1};
use nom::sequence::{delimited, preceded, terminated, tuple};
use nom::{Finish, IResult};

/// Per-`Boost`/`Downrank` step, as a fraction of the score. `Boost(3)` lifts a
/// score by 30% before clamping; `Downrank(3)` cuts it by 30%. An AMATL choice
/// (see the module docs), not Stract's internal scale.
const OPTICS_STEP: f64 = 0.1;

/// Floor a `Downrank` multiplier is clamped to, so a large `n` never zeroes a
/// score outright (which would be indistinguishable from `DiscardNonMatching`).
const DOWNRANK_FLOOR: f64 = 0.01;

/// One field of a result an optics rule can match against.
#[derive(Debug, Clone, PartialEq)]
pub enum MatchField {
    /// Match against the host of the canonical URL (`Site` and `Domain` are
    /// treated identically here — both compare against `host_str()`).
    Site(String),
    /// Alias of [`MatchField::Site`]; kept distinct so a round-trip of a parsed
    /// document preserves which keyword the operator wrote.
    Domain(String),
    /// Match against the result title.
    Title(String),
    /// Match against the full canonical URL string.
    Url(String),
}

/// The adjustment a matching [`Rule`] applies.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Boost(u32),
    Downrank(u32),
}

impl Action {
    /// The multiplier this action applies to a score. See the module docs for
    /// the (AMATL-chosen) scale.
    fn multiplier(&self) -> f64 {
        match self {
            Action::Boost(n) => 1.0 + f64::from(*n) * OPTICS_STEP,
            Action::Downrank(n) => (1.0 - f64::from(*n) * OPTICS_STEP).max(DOWNRANK_FLOOR),
        }
    }
}

/// One `Rule { Matches {..}, Matches {..}, Action(..) }` block. The `matches`
/// are OR-ed: the rule fires if *any* of them matches the result.
#[derive(Debug, Clone, PartialEq)]
pub struct Rule {
    pub matches: Vec<MatchField>,
    pub action: Action,
}

/// A fully parsed `.optic` file.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct OpticsDocument {
    pub rules: Vec<Rule>,
    /// When `true`, a result that matched *no* rule is dropped from the final
    /// set entirely (`DiscardNonMatching;` directive).
    pub discard_non_matching: bool,
}

/// A syntax error from [`parse_optics`], with an approximate 1-based line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpticsParseError {
    /// Approximate 1-based line the parser gave up on.
    pub line: usize,
    /// Human-readable description of what went wrong.
    pub message: String,
}

impl std::fmt::Display for OpticsParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "optics parse error at line {}: {}",
            self.line, self.message
        )
    }
}

impl std::error::Error for OpticsParseError {}

impl OpticsDocument {
    /// Applies every rule to `result` in file order, returning the combined
    /// multiplier and whether *any* rule matched. A `None` multiplier means no
    /// rule matched; the caller pairs that with [`Self::discard_non_matching`].
    pub fn multiplier_for(&self, result: &RankedResult) -> OpticsOutcome {
        let mut multiplier = 1.0_f64;
        let mut matched = false;
        for rule in &self.rules {
            if rule
                .matches
                .iter()
                .any(|field| field_matches(field, result))
            {
                matched = true;
                multiplier *= rule.action.multiplier();
            }
        }
        OpticsOutcome {
            multiplier,
            matched,
        }
    }
}

/// Result of evaluating an [`OpticsDocument`] against one result.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OpticsOutcome {
    /// Product of every matching rule's multiplier (`1.0` if none matched).
    pub multiplier: f64,
    /// Whether at least one rule matched.
    pub matched: bool,
}

// ── Pattern matching (wildcard `*` + `|...|` anchors) ─────────────────────────

/// Matches `pattern` against `haystack`, case-insensitively.
///
/// - A leading `|` anchors the match to the start of `haystack`; a trailing `|`
///   anchors it to the end. Without an anchor the pattern may match a
///   substring.
/// - `*` is a wildcard for zero or more characters.
/// - Every other character is matched literally.
///
/// This is a deliberately small glob, implemented by hand rather than pulling
/// in `regex` (not currently a workspace dependency).
fn pattern_matches(pattern: &str, haystack: &str) -> bool {
    let haystack = haystack.to_lowercase();
    let mut pattern = pattern.to_lowercase();

    let anchored_start = pattern.starts_with('|');
    if anchored_start {
        pattern.remove(0);
    }
    let anchored_end = pattern.ends_with('|');
    if anchored_end {
        pattern.pop();
    }

    // Split on `*`; each fragment must appear in order. `anchored_start`
    // forces the first fragment to sit at position 0, `anchored_end` forces
    // the last to finish at the end.
    let fragments: Vec<&str> = pattern.split('*').collect();
    let has_wildcard = fragments.len() > 1;

    if !has_wildcard {
        // No `*`: behaviour depends purely on the anchors.
        return match (anchored_start, anchored_end) {
            (true, true) => haystack == pattern,
            (true, false) => haystack.starts_with(&pattern),
            (false, true) => haystack.ends_with(&pattern),
            (false, false) => haystack.contains(&pattern),
        };
    }

    let mut cursor = 0usize;
    for (index, fragment) in fragments.iter().enumerate() {
        let is_first = index == 0;
        let is_last = index == fragments.len() - 1;

        if fragment.is_empty() {
            // Leading/trailing/`**` empty fragment: nothing to match here.
            continue;
        }

        match haystack[cursor..].find(fragment) {
            Some(found) => {
                let absolute = cursor + found;
                if is_first && anchored_start && absolute != 0 {
                    return false;
                }
                cursor = absolute + fragment.len();
            }
            None => return false,
        }

        if is_last && anchored_end && cursor != haystack.len() {
            return false;
        }
    }

    // An anchored-start pattern that begins with `*` (empty first fragment)
    // still allows any prefix; an anchored-end pattern ending in `*` allows
    // any suffix. Both are already handled by the `continue` above.
    true
}

fn field_matches(field: &MatchField, result: &RankedResult) -> bool {
    let url = &result.result.canonical_url.0;
    match field {
        MatchField::Site(pattern) | MatchField::Domain(pattern) => url
            .host_str()
            .is_some_and(|host| pattern_matches(pattern, host)),
        MatchField::Title(pattern) => result
            .result
            .title
            .as_deref()
            .is_some_and(|title| pattern_matches(pattern, title)),
        MatchField::Url(pattern) => pattern_matches(pattern, url.as_str()),
    }
}

// ── Parser ───────────────────────────────────────────────────────────────────

/// Whitespace and comments (`// line` and `/* block */`), zero or more.
fn skip_ws(input: &str) -> IResult<&str, ()> {
    value(
        (),
        many0(alt((
            value((), multispace1),
            value(
                (),
                tuple((tag("//"), take_while1(|c| c != '\n'), opt(char('\n')))),
            ),
            // Empty `//` at end of line.
            value((), tuple((tag("//"), opt(char('\n'))))),
            value((), tuple((tag("/*"), take_until("*/"), tag("*/")))),
        ))),
    )(input)
}

/// A token, with surrounding whitespace/comments consumed after it.
fn lexeme<'a, F, O>(mut inner: F) -> impl FnMut(&'a str) -> IResult<&'a str, O>
where
    F: FnMut(&'a str) -> IResult<&'a str, O>,
{
    move |input| {
        let (input, out) = inner(input)?;
        let (input, ()) = skip_ws(input)?;
        Ok((input, out))
    }
}

fn symbol<'a>(s: &'static str) -> impl FnMut(&'a str) -> IResult<&'a str, &'a str> {
    lexeme(tag(s))
}

/// A double-quoted string literal; returns its contents (no escape handling —
/// `.optic` patterns do not use escapes).
fn string_literal(input: &str) -> IResult<&str, String> {
    lexeme(map(
        delimited(char('"'), take_while1(|c| c != '"'), char('"')),
        |s: &str| s.to_string(),
    ))(input)
}

fn unsigned(input: &str) -> IResult<&str, u32> {
    lexeme(map_res(recognize(digit1), |s: &str| s.parse::<u32>()))(input)
}

/// `Site("...")` / `Domain("...")` / `Title("...")` / `Url("...")`.
fn match_field(input: &str) -> IResult<&str, MatchField> {
    alt((
        map(
            preceded(
                symbol("Site"),
                delimited(symbol("("), string_literal, symbol(")")),
            ),
            MatchField::Site,
        ),
        map(
            preceded(
                symbol("Domain"),
                delimited(symbol("("), string_literal, symbol(")")),
            ),
            MatchField::Domain,
        ),
        map(
            preceded(
                symbol("Title"),
                delimited(symbol("("), string_literal, symbol(")")),
            ),
            MatchField::Title,
        ),
        map(
            preceded(
                symbol("Url"),
                delimited(symbol("("), string_literal, symbol(")")),
            ),
            MatchField::Url,
        ),
    ))(input)
}

/// `Matches { <field> }` — one field per block (real `.optic` files put one
/// match per `Matches {}` and OR several blocks within a `Rule`).
fn matches_block(input: &str) -> IResult<&str, MatchField> {
    preceded(
        symbol("Matches"),
        delimited(symbol("{"), match_field, symbol("}")),
    )(input)
}

/// `Action(Boost(n))` / `Action(Downrank(n))`.
fn action(input: &str) -> IResult<&str, Action> {
    preceded(
        symbol("Action"),
        delimited(
            symbol("("),
            alt((
                map(
                    preceded(
                        symbol("Boost"),
                        delimited(symbol("("), unsigned, symbol(")")),
                    ),
                    Action::Boost,
                ),
                map(
                    preceded(
                        symbol("Downrank"),
                        delimited(symbol("("), unsigned, symbol(")")),
                    ),
                    Action::Downrank,
                ),
            )),
            symbol(")"),
        ),
    )(input)
}

/// `Rule { Matches {..}, Matches {..}, Action(..) };`
fn rule(input: &str) -> IResult<&str, Rule> {
    let (input, _) = symbol("Rule")(input)?;
    let (input, _) = symbol("{")(input)?;
    // Comma-separated list of `Matches {}` blocks and exactly one `Action(..)`.
    let (input, matches) = many1(terminated(matches_block, opt(symbol(","))))(input)?;
    let (input, act) = action(input)?;
    let (input, _) = opt(symbol(","))(input)?;
    let (input, _) = symbol("}")(input)?;
    let (input, _) = opt(symbol(";"))(input)?;
    Ok((
        input,
        Rule {
            matches,
            action: act,
        },
    ))
}

/// One top-level item: a `Rule` or the bare `DiscardNonMatching;` directive.
enum Item {
    Rule(Rule),
    DiscardNonMatching,
}

fn item(input: &str) -> IResult<&str, Item> {
    alt((
        map(
            terminated(symbol("DiscardNonMatching"), opt(symbol(";"))),
            |_| Item::DiscardNonMatching,
        ),
        map(rule, Item::Rule),
    ))(input)
}

/// Parses a `.optic` document. Rules and `DiscardNonMatching` may appear in any
/// order and be interleaved. Returns a typed error (with an approximate line)
/// rather than silently yielding an empty document on bad syntax.
pub fn parse_optics(input: &str) -> Result<OpticsDocument, OpticsParseError> {
    let (rest, ()) = skip_ws(input).map_err(|_| OpticsParseError {
        line: 1,
        message: "failed to scan leading whitespace".to_string(),
    })?;

    let parsed: IResult<&str, Vec<Item>> = many0(item)(rest);
    let (rest, items) =
        parsed
            .finish()
            .map_err(|error: nom::error::Error<&str>| OpticsParseError {
                line: line_of(input, error.input),
                message: format!("unexpected token near {:?}", snippet(error.input)),
            })?;

    if !rest.trim().is_empty() {
        return Err(OpticsParseError {
            line: line_of(input, rest),
            message: format!("trailing content that is not a Rule: {:?}", snippet(rest)),
        });
    }

    let mut document = OpticsDocument::default();
    for parsed_item in items {
        match parsed_item {
            Item::Rule(rule) => document.rules.push(rule),
            Item::DiscardNonMatching => document.discard_non_matching = true,
        }
    }
    Ok(document)
}

/// 1-based line number of `remainder` within `whole` (both slices of the same
/// original string, `remainder` a suffix of `whole`).
fn line_of(whole: &str, remainder: &str) -> usize {
    let consumed = whole.len().saturating_sub(remainder.len());
    whole[..consumed.min(whole.len())]
        .bytes()
        .filter(|&b| b == b'\n')
        .count()
        + 1
}

fn snippet(input: &str) -> String {
    input.chars().take(24).collect::<String>()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        CanonicalUrl, DeduplicatedResult, DuplicateStatus, OriginalUrl, ResultType,
    };
    use crate::ranking::rank;
    use crate::{Rank, RankingPolicyV1, SCHEMA_VERSION};
    use std::collections::BTreeMap;

    /// Builds a real [`RankedResult`] by running `rank()` on one synthetic
    /// input, then overwriting its canonical URL and title so a test can
    /// control what the optics matcher sees. Reusing `rank()` keeps the
    /// `RankingExplanation` construction identical to production (mirrors the
    /// helper in `ranking_adjustments`).
    fn ranked(host_and_path: &str, title: &str) -> RankedResult {
        let url = url::Url::parse(&format!("https://{host_and_path}")).unwrap();
        let seed = url::Url::parse("https://seed.example.org/page").unwrap();
        let item = DeduplicatedResult {
            schema_version: SCHEMA_VERSION.into(),
            title: Some("seed".into()),
            original_url: OriginalUrl(seed.clone()),
            canonical_url: CanonicalUrl(seed.clone()),
            original_urls: vec![OriginalUrl(seed)],
            providers: vec!["p".into()],
            representative_provider: "p".into(),
            provider_ranks: BTreeMap::from([("p".to_string(), Rank::new(1).ok())]),
            snippet: Some("snippet".into()),
            alternate_snippets: vec![],
            result_type: ResultType::Organic,
            published_at: None,
            author: None,
            language: None,
            file_type: None,
            thumbnail: None,
            metadata: BTreeMap::new(),
            observed_dates: vec![],
            duplicate_status: DuplicateStatus::Distinct,
            merge_reason: None,
            possible_duplicate_with: vec![],
        };
        let query = crate::parse_query("seed".into()).unwrap();
        let mut ranked = rank(
            &query,
            "2026-08-12T00:00:00Z",
            1,
            vec![item],
            &RankingPolicyV1::default(),
        );
        let mut r = ranked.remove(0);
        r.result.canonical_url = CanonicalUrl(url);
        r.result.title = Some(title.to_string());
        r
    }

    // ── Parsing real syntax ──────────────────────────────────────────────

    #[test]
    fn parses_a_reduced_devdocs_optic() {
        // A 2-rule reduction of the shape of Stract's real devdocs.optic:
        // several `Matches {}` blocks OR-ed within one `Rule`, one `Action`.
        let source = r#"
            // Boost official documentation sites.
            Rule {
                Matches {
                    Site("|developer.mozilla.org|")
                },
                Matches {
                    Domain("|docs.rs|")
                },
                Action(Boost(3))
            };
            /* Downrank listicle spam */
            Rule {
                Matches {
                    Title("Top * of 2022")
                },
                Action(Downrank(3))
            };
        "#;
        let doc = parse_optics(source).expect("should parse");
        assert_eq!(doc.rules.len(), 2);
        assert_eq!(
            doc.rules[0].matches,
            vec![
                MatchField::Site("|developer.mozilla.org|".into()),
                MatchField::Domain("|docs.rs|".into()),
            ]
        );
        assert_eq!(doc.rules[0].action, Action::Boost(3));
        assert_eq!(doc.rules[1].action, Action::Downrank(3));
        assert!(!doc.discard_non_matching);
    }

    #[test]
    fn discard_non_matching_is_recognised_before_any_rule() {
        let doc = parse_optics(
            r#"
            DiscardNonMatching;
            Rule { Matches { Site("|example.com|") }, Action(Boost(1)) };
        "#,
        )
        .unwrap();
        assert!(doc.discard_non_matching);
        assert_eq!(doc.rules.len(), 1);
    }

    #[test]
    fn discard_non_matching_is_recognised_after_and_between_rules() {
        let doc = parse_optics(
            r#"
            Rule { Matches { Site("|a.com|") }, Action(Boost(1)) };
            DiscardNonMatching;
            Rule { Matches { Site("|b.com|") }, Action(Downrank(1)) };
        "#,
        )
        .unwrap();
        assert!(doc.discard_non_matching);
        assert_eq!(doc.rules.len(), 2);
    }

    #[test]
    fn empty_document_is_ok_and_inert() {
        let doc = parse_optics("   // just a comment\n").unwrap();
        assert_eq!(doc, OpticsDocument::default());
    }

    #[test]
    fn invalid_syntax_is_a_typed_error_not_a_panic() {
        let err = parse_optics(
            r#"
            Rule {
                Matches { Site("|ok.com|") },
                Action(Explode(9))
            };
        "#,
        )
        .expect_err("Explode is not a valid action");
        assert!(err.line >= 2, "line should point past the first line");
        assert!(err.to_string().contains("optics parse error"));
    }

    #[test]
    fn unknown_match_field_is_a_typed_error() {
        let err = parse_optics(r#"Rule { Matches { Schema("Recipe") }, Action(Boost(1)) };"#)
            .expect_err("Schema is out of scope");
        assert!(!err.message.is_empty());
    }

    // ── Pattern matching ─────────────────────────────────────────────────

    #[test]
    fn anchored_host_pattern_is_exact() {
        assert!(pattern_matches(
            "|developer.mozilla.org|",
            "developer.mozilla.org"
        ));
        assert!(!pattern_matches(
            "|developer.mozilla.org|",
            "evil-developer.mozilla.org"
        ));
        assert!(!pattern_matches(
            "|developer.mozilla.org|",
            "developer.mozilla.org.attacker.test"
        ));
    }

    #[test]
    fn leading_anchor_only_is_a_prefix_match() {
        assert!(pattern_matches("|docs.", "docs.rs"));
        assert!(!pattern_matches("|docs.", "the-docs.rs"));
    }

    #[test]
    fn wildcard_matches_title_like_the_quickstart_example() {
        assert!(pattern_matches(
            "Top * of 2022",
            "Top 10 Frameworks of 2022"
        ));
        assert!(pattern_matches("Top * of 2022", "Top Picks of 2022"));
        assert!(!pattern_matches(
            "Top * of 2022",
            "Top 10 Frameworks of 2023"
        ));
    }

    #[test]
    fn wildcard_is_case_insensitive() {
        assert!(pattern_matches("Top * of 2022", "TOP 5 OF 2022"));
    }

    #[test]
    fn anchored_wildcard_pattern() {
        assert!(pattern_matches(
            "|https://docs.rs/*|",
            "https://docs.rs/serde/latest"
        ));
        assert!(!pattern_matches(
            "|https://docs.rs/*|",
            "https://example.com/https://docs.rs/x"
        ));
    }

    // ── Evaluation against a RankedResult ────────────────────────────────

    #[test]
    fn site_rule_matches_by_host_and_produces_a_boost_multiplier() {
        let doc =
            parse_optics(r#"Rule { Matches { Site("|docs.rs|") }, Action(Boost(5)) };"#).unwrap();
        let outcome = doc.multiplier_for(&ranked("docs.rs/serde", "serde"));
        assert!(outcome.matched);
        assert!((outcome.multiplier - 1.5).abs() < 1e-9);
    }

    #[test]
    fn downrank_multiplier_is_below_one_and_floored() {
        let doc =
            parse_optics(r#"Rule { Matches { Title("Top * of 2022") }, Action(Downrank(3)) };"#)
                .unwrap();
        let hit = doc.multiplier_for(&ranked("blog.test/x", "Top 10 of 2022"));
        assert!(hit.matched);
        assert!((hit.multiplier - 0.7).abs() < 1e-9);

        let big =
            parse_optics(r#"Rule { Matches { Title("*") }, Action(Downrank(100)) };"#).unwrap();
        assert!(
            (big.multiplier_for(&ranked("x.test/y", "anything"))
                .multiplier
                - DOWNRANK_FLOOR)
                .abs()
                < 1e-9
        );
    }

    #[test]
    fn non_matching_result_has_unit_multiplier_and_matched_false() {
        let doc =
            parse_optics(r#"Rule { Matches { Site("|docs.rs|") }, Action(Boost(5)) };"#).unwrap();
        let outcome = doc.multiplier_for(&ranked("example.com/page", "unrelated"));
        assert!(!outcome.matched);
        assert_eq!(outcome.multiplier, 1.0);
    }

    #[test]
    fn multiple_matching_rules_compound() {
        let doc = parse_optics(
            r#"
            Rule { Matches { Site("|docs.rs|") }, Action(Boost(5)) };
            Rule { Matches { Url("*serde*") }, Action(Boost(5)) };
        "#,
        )
        .unwrap();
        let outcome = doc.multiplier_for(&ranked("docs.rs/serde/latest", "serde"));
        assert!((outcome.multiplier - (1.5 * 1.5)).abs() < 1e-9);
    }
}
