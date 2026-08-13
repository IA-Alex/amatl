use crate::model::{Category, Classification, Query, SCHEMA_VERSION};
use std::collections::BTreeMap;

pub fn classify(query: &Query) -> Classification {
    let text = query.normalized_query.to_lowercase();
    let mut signals = BTreeMap::<Category, (f64, Vec<String>)>::new();

    for file_type in &query.file_types {
        if ["rs", "py", "js", "ts", "java", "go", "c", "cpp"].contains(&file_type.as_str()) {
            add_signal(
                &mut signals,
                Category::Code,
                0.98,
                "explicit_code_file_filter",
            );
        } else {
            add_signal(
                &mut signals,
                Category::Documentation,
                0.95,
                "explicit_file_filter",
            );
        }
    }
    for domain in query.domains.iter().map(|value| value.to_ascii_lowercase()) {
        if domain.contains("github.") || domain.contains("gitlab.") {
            add_signal(&mut signals, Category::Code, 0.97, "explicit_code_domain");
        } else if domain.contains("arxiv.") || domain.contains("doi.") {
            add_signal(
                &mut signals,
                Category::Academic,
                0.97,
                "explicit_academic_domain",
            );
        } else if domain.contains("reddit.") || domain.contains("stackoverflow.") {
            add_signal(&mut signals, Category::Forum, 0.97, "explicit_forum_domain");
        } else if domain.contains("youtube.") || domain.contains("vimeo.") {
            add_signal(&mut signals, Category::Media, 0.97, "explicit_media_domain");
        } else if domain.contains("linkedin.")
            || domain.contains("instagram.")
            || domain.contains("twitter.")
            || domain == "x.com"
        {
            add_signal(
                &mut signals,
                Category::Social,
                0.97,
                "explicit_social_domain",
            );
        }
    }

    lexical_signal(
        &mut signals,
        &text,
        Category::Navigation,
        0.88,
        "navigation_lexical_signal",
        &[
            "official website",
            "homepage",
            "login",
            "sign in",
            "navigate to",
        ],
    );
    lexical_signal(
        &mut signals,
        &text,
        Category::Code,
        0.86,
        "code_lexical_signal",
        &[
            "source code",
            "code example",
            "stack trace",
            "compile error",
            "github",
            "function",
            "class",
            "crate",
        ],
    );
    lexical_signal(
        &mut signals,
        &text,
        Category::Documentation,
        0.84,
        "documentation_lexical_signal",
        &[
            "documentation",
            "manual",
            "api reference",
            "reference guide",
            "docs",
        ],
    );
    lexical_signal(
        &mut signals,
        &text,
        Category::Academic,
        0.86,
        "academic_lexical_signal",
        &[
            "research paper",
            "peer reviewed",
            "journal",
            "arxiv",
            "doi",
            "systematic review",
            "study",
        ],
    );
    lexical_signal(
        &mut signals,
        &text,
        Category::News,
        0.84,
        "news_lexical_signal",
        &["news", "today", "latest", "breaking", "current events"],
    );
    lexical_signal(
        &mut signals,
        &text,
        Category::Commercial,
        0.82,
        "commercial_lexical_signal",
        &[
            "buy",
            "price",
            "pricing",
            "shop",
            "discount",
            "product comparison",
        ],
    );
    lexical_signal(
        &mut signals,
        &text,
        Category::Forum,
        0.80,
        "forum_lexical_signal",
        &[
            "forum",
            "discussion",
            "reddit",
            "stackoverflow",
            "community answer",
        ],
    );
    lexical_signal(
        &mut signals,
        &text,
        Category::Social,
        0.80,
        "social_lexical_signal",
        &[
            "social media",
            "linkedin",
            "instagram",
            "twitter",
            "mastodon",
        ],
    );
    lexical_signal(
        &mut signals,
        &text,
        Category::Media,
        0.80,
        "media_lexical_signal",
        &[
            "video",
            "podcast",
            "image",
            "photo",
            "youtube",
            "livestream",
        ],
    );
    lexical_signal(
        &mut signals,
        &text,
        Category::Technical,
        0.76,
        "technical_lexical_signal",
        &[
            "rust",
            "python",
            "compiler",
            "database",
            "algorithm",
            "protocol",
            "software",
            "api",
        ],
    );

    if signals.is_empty() {
        signals.insert(Category::General, (0.50, vec!["general_fallback".into()]));
    }

    let mut ordered = signals
        .iter()
        .map(|(category, (confidence, reasons))| (category.clone(), *confidence, reasons.clone()))
        .collect::<Vec<_>>();
    ordered.sort_by(|left, right| {
        right
            .1
            .total_cmp(&left.1)
            .then_with(|| left.0.cmp(&right.0))
    });
    let primary_category = ordered[0].0.clone();
    let confidence = ordered[0].1;
    let secondary_categories = ordered
        .iter()
        .skip(1)
        .take(2)
        .map(|value| value.0.clone())
        .collect();
    let confidence_by_category = ordered
        .iter()
        .take(3)
        .map(|(category, confidence, _)| (category.clone(), *confidence))
        .collect();
    let reasons = ordered
        .iter()
        .take(3)
        .flat_map(|(_, _, reasons)| reasons.iter().cloned())
        .collect();

    Classification {
        schema_version: SCHEMA_VERSION.into(),
        primary_category,
        secondary_categories,
        confidence,
        confidence_by_category,
        reasons,
    }
}

fn add_signal(
    signals: &mut BTreeMap<Category, (f64, Vec<String>)>,
    category: Category,
    confidence: f64,
    reason: &str,
) {
    let entry = signals.entry(category).or_insert((confidence, Vec::new()));
    entry.0 = entry.0.max(confidence);
    if !entry.1.iter().any(|value| value == reason) {
        entry.1.push(reason.into());
    }
}

fn lexical_signal(
    signals: &mut BTreeMap<Category, (f64, Vec<String>)>,
    text: &str,
    category: Category,
    confidence: f64,
    reason: &str,
    patterns: &[&str],
) {
    if patterns
        .iter()
        .any(|pattern| contains_pattern(text, pattern))
    {
        add_signal(signals, category, confidence, reason);
    }
}

fn contains_pattern(text: &str, pattern: &str) -> bool {
    if pattern.contains(' ') {
        return text.contains(pattern);
    }
    text.split(|character: char| !character.is_alphanumeric() && character != '_')
        .any(|token| token == pattern)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_query;

    #[test]
    fn classification_is_deterministic_and_can_emit_secondaries() {
        let query = parse_query("rust source code documentation".into()).unwrap();
        let classification = classify(&query);
        assert_eq!(classification, classify(&query));
        assert_eq!(classification.primary_category, Category::Code);
        assert!(classification
            .secondary_categories
            .contains(&Category::Documentation));
        assert_eq!(
            classification.confidence_by_category[&Category::Code],
            classification.confidence
        );
    }

    #[test]
    fn every_contract_category_is_reachable() {
        let cases = [
            ("ordinary question", Category::General),
            ("database protocol", Category::Technical),
            ("source code example", Category::Code),
            ("api reference guide", Category::Documentation),
            ("latest news", Category::News),
            ("peer reviewed journal", Category::Academic),
            ("product comparison price", Category::Commercial),
            ("community forum discussion", Category::Forum),
            ("social media linkedin", Category::Social),
            ("video podcast", Category::Media),
            ("official website login", Category::Navigation),
        ];
        for (raw, expected) in cases {
            assert_eq!(
                classify(&parse_query(raw.into()).unwrap()).primary_category,
                expected,
                "{raw}"
            );
        }
    }

    #[test]
    fn explicit_filter_has_priority_over_lexical_signal() {
        let query = parse_query("latest news filetype:rs".into()).unwrap();
        assert_eq!(classify(&query).primary_category, Category::Code);
    }
}
