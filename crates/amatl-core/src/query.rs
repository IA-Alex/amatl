use crate::model::{Query, QueryWarning, SCHEMA_VERSION};
use thiserror::Error;

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum QueryParseError {
    #[error("query must not be empty")]
    Empty,
}

pub fn parse_query(raw_query: String) -> Result<Query, QueryParseError> {
    if raw_query.trim().is_empty() {
        return Err(QueryParseError::Empty);
    }

    let mut query = Query {
        schema_version: SCHEMA_VERSION.into(),
        raw_query: raw_query.clone(),
        normalized_query: String::new(),
        quoted_terms: Vec::new(),
        excluded_terms: Vec::new(),
        domains: Vec::new(),
        excluded_domains: Vec::new(),
        file_types: Vec::new(),
        language: None,
        region: None,
        date_from: None,
        date_to: None,
        warnings: Vec::new(),
    };
    let mut free: Vec<String> = Vec::new();

    for token in tokenize(&raw_query) {
        let token = token.as_str();
        let lower = token.to_ascii_lowercase();
        let (operator, _) = lower.split_once(':').unwrap_or(("", ""));
        let original_value = token.split_once(':').map(|(_, value)| value).unwrap_or("");
        match operator {
            "site" if valid_domain(original_value) => {
                push_unique(&mut query.domains, original_value.to_ascii_lowercase())
            }
            "-site" if valid_domain(original_value) => push_unique(
                &mut query.excluded_domains,
                original_value.to_ascii_lowercase(),
            ),
            "filetype" if valid_file_type(original_value) => push_unique(
                &mut query.file_types,
                original_value.trim_start_matches('.').to_ascii_lowercase(),
            ),
            "lang" if valid_language(original_value) => replace_filter(
                &mut query.language,
                original_value.to_ascii_lowercase(),
                "repeated_language_filter",
                "lang",
                &mut query.warnings,
            ),
            "region" if valid_region(original_value) => replace_filter(
                &mut query.region,
                original_value.to_ascii_uppercase(),
                "repeated_region_filter",
                "region",
                &mut query.warnings,
            ),
            "after" if valid_date(original_value) => query.date_from = Some(original_value.into()),
            "before" if valid_date(original_value) => query.date_to = Some(original_value.into()),
            "exact" if !original_value.is_empty() => query
                .quoted_terms
                .push(original_value.trim_matches('"').into()),
            _ if !operator.is_empty()
                && matches!(
                    operator,
                    "site"
                        | "-site"
                        | "filetype"
                        | "lang"
                        | "region"
                        | "after"
                        | "before"
                        | "exact"
                ) =>
            {
                query.warnings.push(QueryWarning {
                    code: "invalid_filter_value".into(),
                    operator: Some(operator.into()),
                    value: Some(original_value.into()),
                    message: "filter was treated as literal text".into(),
                });
                free.push(token.to_owned());
            }
            _ if token.starts_with('-') && token.len() > 1 => {
                query.excluded_terms.push(token[1..].into())
            }
            _ if token.starts_with('"') && token.ends_with('"') && token.len() > 2 => {
                query.quoted_terms.push(token.trim_matches('"').into())
            }
            _ => free.push(token.to_owned()),
        }
    }

    query.domains.retain(|domain| {
        if query.excluded_domains.contains(domain) {
            query.warnings.push(QueryWarning {
                code: "domain_included_and_excluded".into(),
                operator: Some("site".into()),
                value: Some(domain.clone()),
                message: "domain exclusion takes precedence".into(),
            });
            false
        } else {
            true
        }
    });
    if matches!((&query.date_from, &query.date_to), (Some(from), Some(to)) if from >= to) {
        query.date_from = None;
        query.date_to = None;
        query.warnings.push(QueryWarning {
            code: "invalid_date_range".into(),
            operator: None,
            value: None,
            message: "after must be earlier than before".into(),
        });
    }
    query.normalized_query = free.join(" ");
    Ok(query)
}

fn push_unique(values: &mut Vec<String>, value: String) {
    if !values.contains(&value) {
        values.push(value);
    }
}

fn replace_filter(
    target: &mut Option<String>,
    value: String,
    code: &str,
    operator: &str,
    warnings: &mut Vec<QueryWarning>,
) {
    if target.replace(value.clone()).is_some() {
        warnings.push(QueryWarning {
            code: code.into(),
            operator: Some(operator.into()),
            value: Some(value),
            message: "last valid filter wins".into(),
        });
    }
}

fn valid_domain(value: &str) -> bool {
    !value.is_empty() && !value.contains(['/', ':']) && value.contains('.')
}
fn valid_file_type(value: &str) -> bool {
    let value = value.trim_start_matches('.');
    !value.is_empty() && value.len() <= 16 && value.chars().all(|c| c.is_ascii_alphanumeric())
}
fn valid_language(value: &str) -> bool {
    (2..=15).contains(&value.len()) && value.chars().all(|c| c.is_ascii_alphabetic() || c == '-')
}
fn valid_region(value: &str) -> bool {
    value.len() == 2 && value.chars().all(|c| c.is_ascii_alphabetic())
}
fn valid_date(value: &str) -> bool {
    let parts: Vec<_> = value.split('-').collect();
    if !(parts.len() == 3
        && parts[0].len() == 4
        && parts[1].len() == 2
        && parts[2].len() == 2
        && parts
            .iter()
            .all(|part| part.chars().all(|c| c.is_ascii_digit())))
    {
        return false;
    }
    let Ok(year) = parts[0].parse::<u32>() else {
        return false;
    };
    let Ok(month) = parts[1].parse::<u32>() else {
        return false;
    };
    let Ok(day) = parts[2].parse::<u32>() else {
        return false;
    };
    let leap = year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
    let max_day = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    (1..=max_day).contains(&day)
}

fn tokenize(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    for character in input.chars() {
        match character {
            '"' => {
                quoted = !quoted;
                current.push(character);
            }
            character if character.is_whitespace() && !quoted => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(character),
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_raw_and_extracts_filters() {
        let raw = "rust site:docs.rs filetype:PDF -old".to_string();
        let query = parse_query(raw.clone()).unwrap();
        assert_eq!(query.raw_query, raw);
        assert_eq!(query.normalized_query, "rust");
        assert_eq!(query.domains, ["docs.rs"]);
        assert_eq!(query.file_types, ["pdf"]);
        assert_eq!(query.excluded_terms, ["old"]);
    }

    #[test]
    fn invalid_filter_becomes_literal_warning() {
        let query = parse_query("rust site:https://docs.rs/a".into()).unwrap();
        assert_eq!(query.normalized_query, "rust site:https://docs.rs/a");
        assert_eq!(query.warnings[0].code, "invalid_filter_value");
    }

    #[test]
    fn rejects_empty_query() {
        assert_eq!(parse_query("  ".into()), Err(QueryParseError::Empty));
    }

    #[test]
    fn keeps_quoted_phrase_together_and_rejects_invalid_calendar_date() {
        let query = parse_query("\"rust async\" before:2026-02-30".into()).unwrap();
        assert_eq!(query.quoted_terms, ["rust async"]);
        assert_eq!(query.date_to, None);
        assert_eq!(query.normalized_query, "before:2026-02-30");
        assert_eq!(query.warnings.len(), 1);
    }
}
