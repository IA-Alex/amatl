use crate::model::{
    Degradation, FieldProvenance, NormalizedResult, OriginalUrl, ProviderResult, ResultType,
    SCHEMA_VERSION,
};
use crate::security::validate_search_url;
use std::collections::BTreeMap;
use time::format_description::well_known::Rfc3339;
use time::macros::format_description;
use time::{Date, OffsetDateTime, PrimitiveDateTime, Time};

pub fn normalize(results: &[ProviderResult]) -> (Vec<NormalizedResult>, Vec<Degradation>) {
    let mut normalized = Vec::new();
    let mut pipeline_degradations = Vec::new();
    for provider_result in results {
        for item in &provider_result.results {
            let url = match validate_search_url(&item.url) {
                Ok(url) => url,
                Err(code) => {
                    pipeline_degradations.push(degradation(
                        code,
                        "provider item discarded due to URL contract",
                    ));
                    continue;
                }
            };
            let mut item_degradations = Vec::new();
            let title = clean_field(&item.title, "title", &mut item_degradations);
            let snippet = clean_field(&item.snippet, "snippet", &mut item_degradations);
            let author = clean_field(&item.author, "author", &mut item_degradations);
            let language = clean_field(&item.language, "language", &mut item_degradations);
            let file_type = clean_field(&item.file_type, "file_type", &mut item_degradations);
            let thumbnail = item.thumbnail.as_ref().and_then(|value| {
                validate_search_url(value)
                    .ok()
                    .map(|_| value.clone())
                    .or_else(|| {
                        item_degradations.push(degradation(
                            "invalid_thumbnail_url",
                            "invalid thumbnail was removed",
                        ));
                        None
                    })
            });
            let mut metadata = item
                .metadata
                .iter()
                .filter_map(|(key, value)| {
                    clean_text(value)
                        .map(|clean| (key.clone(), clean))
                        .or_else(|| {
                            item_degradations.push(degradation(
                                "invalid_metadata_field",
                                "invalid metadata field was removed",
                            ));
                            None
                        })
                })
                .collect::<BTreeMap<_, _>>();
            let published_at = item.published_at.as_ref().and_then(|value| {
                normalize_date(value).or_else(|| {
                    metadata.insert("reported_published_at_invalid".into(), value.clone());
                    item_degradations.push(degradation(
                        "invalid_published_at",
                        "invalid published date was preserved as metadata",
                    ));
                    None
                })
            });
            let mut provenance = BTreeMap::from([
                ("url".into(), FieldProvenance::Reported),
                (
                    "result_type".into(),
                    if item.result_type.is_some() {
                        FieldProvenance::Reported
                    } else {
                        FieldProvenance::Derived
                    },
                ),
            ]);
            for (field, present) in [
                ("title", title.is_some()),
                ("snippet", snippet.is_some()),
                ("published_at", published_at.is_some()),
                ("author", author.is_some()),
                ("language", language.is_some()),
                ("file_type", file_type.is_some()),
                ("thumbnail", thumbnail.is_some()),
                ("metadata", !metadata.is_empty()),
            ] {
                if present {
                    provenance.insert(field.into(), FieldProvenance::Reported);
                }
            }
            normalized.push(NormalizedResult {
                schema_version: SCHEMA_VERSION.into(),
                title,
                raw_url: item.url.clone(),
                url: OriginalUrl(url),
                provider: provider_result.provider.clone(),
                provider_rank: item.provider_rank,
                snippet,
                result_type: item.result_type.clone().unwrap_or(ResultType::Organic),
                published_at,
                author,
                language,
                file_type,
                thumbnail,
                metadata,
                provenance,
                degradations: item_degradations,
            });
        }
    }
    (normalized, pipeline_degradations)
}

fn clean_field(
    value: &Option<String>,
    field: &str,
    degradations: &mut Vec<Degradation>,
) -> Option<String> {
    value.as_ref().and_then(|value| {
        clean_text(value).or_else(|| {
            degradations.push(degradation(
                &format!("invalid_{field}"),
                &format!("invalid {field} was removed"),
            ));
            None
        })
    })
}

fn clean_text(value: &str) -> Option<String> {
    if value.contains(['\0', '\u{fffd}'])
        || value
            .chars()
            .any(|character| character.is_control() && !character.is_whitespace())
    {
        return None;
    }
    let clean = decode_html_entities(value)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    (!clean.is_empty()).then_some(clean)
}

fn decode_html_entities(value: &str) -> String {
    let mut output = value
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&nbsp;", " ");
    while let Some(start) = output.find("&#") {
        let Some(relative_end) = output[start..].find(';') else {
            break;
        };
        let end = start + relative_end;
        let entity = &output[start + 2..end];
        let code = entity
            .strip_prefix(['x', 'X'])
            .and_then(|hex| u32::from_str_radix(hex, 16).ok())
            .or_else(|| entity.parse::<u32>().ok());
        let Some(character) = code.and_then(char::from_u32) else {
            break;
        };
        output.replace_range(start..=end, &character.to_string());
    }
    output
}

fn normalize_date(value: &str) -> Option<String> {
    if let Ok(date_time) = OffsetDateTime::parse(value, &Rfc3339) {
        return date_time.format(&Rfc3339).ok();
    }
    let date = Date::parse(value, format_description!("[year]-[month]-[day]")).ok()?;
    PrimitiveDateTime::new(date, Time::MIDNIGHT)
        .assume_utc()
        .format(&Rfc3339)
        .ok()
}

fn degradation(code: &str, message: &str) -> Degradation {
    Degradation {
        code: code.into(),
        component: "normalization".into(),
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ProviderExecutionStatus, ProviderItem};

    fn item(url: &str) -> ProviderItem {
        ProviderItem {
            title: None,
            url: url.into(),
            provider_rank: None,
            snippet: None,
            result_type: None,
            published_at: None,
            author: None,
            language: None,
            file_type: None,
            thumbnail: None,
            metadata: BTreeMap::new(),
        }
    }

    fn provider(items: Vec<ProviderItem>) -> ProviderResult {
        ProviderResult {
            schema_version: SCHEMA_VERSION.into(),
            provider: "p".into(),
            status: ProviderExecutionStatus::Success,
            results: items,
            accepted_filters: vec![],
            ignored_filters: vec![],
            approximated_filters: vec![],
            errors: vec![],
        }
    }

    #[test]
    fn drops_invalid_url_but_keeps_valid_item() {
        let mut valid = item("https://example.com");
        valid.title = Some("  valid &amp; title ".into());
        let (items, degradations) = normalize(&[provider(vec![item("bad"), valid])]);
        assert_eq!(items.len(), 1);
        assert_eq!(degradations.len(), 1);
        assert_eq!(items[0].title.as_deref(), Some("valid & title"));
        assert_eq!(items[0].result_type, ResultType::Organic);
        assert_eq!(items[0].provenance["result_type"], FieldProvenance::Derived);
    }

    #[test]
    fn corrupt_snippet_and_invalid_date_degrade_only_fields() {
        let mut value = item("https://example.com");
        value.snippet = Some("bad\u{fffd}snippet".into());
        value.published_at = Some("2026-02-30".into());
        let (items, _) = normalize(&[provider(vec![value])]);
        assert_eq!(items.len(), 1);
        assert!(items[0].snippet.is_none());
        assert!(items[0].published_at.is_none());
        assert_eq!(
            items[0].metadata["reported_published_at_invalid"],
            "2026-02-30"
        );
        assert_eq!(items[0].degradations.len(), 2);
    }
}
