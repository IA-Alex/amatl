use crate::model::{
    CanonicalResult, CanonicalTransformation, CanonicalUrl, CanonicalizationStatus,
    NormalizedResult, SCHEMA_VERSION,
};

const TRACKING_KEYS: &[&str] = &[
    "fbclid", "gclid", "msclkid", "yclid", "_ga", "_gl", "mc_cid", "mc_eid",
];

pub fn canonicalize(result: NormalizedResult) -> CanonicalResult {
    let mut url = result.url.0.clone();
    let mut transformations = structural_transformations(&result.raw_url, &url);
    let malformed_percent_encoding = has_malformed_percent_encoding(&result.raw_url);

    let normalized_percent = normalize_percent_hex(url.as_str());
    if normalized_percent != url.as_str() {
        if let Ok(normalized) = url::Url::parse(&normalized_percent) {
            url = normalized;
            transformations.push(CanonicalTransformation::NormalizePercentHex);
        }
    }

    if let Some(query) = url.query().map(str::to_string) {
        let mut kept = Vec::new();
        for segment in query.split('&') {
            let key = segment.split_once('=').map_or(segment, |(key, _)| key);
            let lower = key.to_ascii_lowercase();
            if lower.starts_with("utm_") || TRACKING_KEYS.contains(&lower.as_str()) {
                transformations.push(CanonicalTransformation::RemoveTrackingParameter(key.into()));
            } else {
                kept.push(segment);
            }
        }
        url.set_query((!kept.is_empty()).then(|| kept.join("&")).as_deref());
    }

    if url.fragment() == Some("") {
        url.set_fragment(None);
        transformations.push(CanonicalTransformation::RemoveEmptyFragment);
    }

    let mut degradations = result.degradations;
    let canonicalization_status = if malformed_percent_encoding {
        degradations.push(crate::Degradation {
            code: "canonicalization_incomplete".into(),
            component: "canonicalization".into(),
            message: "malformed percent escape was preserved conservatively".into(),
        });
        CanonicalizationStatus::Degraded
    } else {
        CanonicalizationStatus::Complete
    };

    CanonicalResult {
        schema_version: SCHEMA_VERSION.into(),
        title: result.title,
        original_url: result.url,
        canonical_url: CanonicalUrl(url),
        provider: result.provider,
        provider_rank: result.provider_rank,
        snippet: result.snippet,
        result_type: result.result_type,
        published_at: result.published_at,
        author: result.author,
        language: result.language,
        file_type: result.file_type,
        thumbnail: result.thumbnail,
        metadata: result.metadata,
        provenance: result.provenance,
        transformations,
        canonicalization_status,
        degradations,
    }
}

fn has_malformed_percent_encoding(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.iter().enumerate().any(|(index, byte)| {
        *byte == b'%'
            && (index + 2 >= bytes.len()
                || !bytes[index + 1].is_ascii_hexdigit()
                || !bytes[index + 2].is_ascii_hexdigit())
    })
}

fn structural_transformations(raw: &str, parsed: &url::Url) -> Vec<CanonicalTransformation> {
    let mut transformations = Vec::new();
    if raw
        .split_once(':')
        .is_some_and(|(scheme, _)| scheme != scheme.to_ascii_lowercase())
    {
        transformations.push(CanonicalTransformation::LowercaseScheme);
    }
    if let Some(raw_host) = raw_host(raw) {
        if !raw_host.is_ascii() {
            transformations.push(CanonicalTransformation::IdnToPunycode);
        } else if parsed
            .host_str()
            .is_some_and(|host| raw_host != host && raw_host.eq_ignore_ascii_case(host))
        {
            transformations.push(CanonicalTransformation::LowercaseHost);
        }
    }
    if has_default_port(raw, parsed.scheme()) {
        transformations.push(CanonicalTransformation::RemoveDefaultPort);
    }
    if has_empty_path(raw) {
        transformations.push(CanonicalTransformation::AddRootPath);
    }
    transformations
}

fn raw_host(raw: &str) -> Option<&str> {
    let authority = raw.split_once("://")?.1.split(['/', '?', '#']).next()?;
    let host_port = authority.rsplit('@').next()?;
    if host_port.starts_with('[') {
        return None;
    }
    Some(
        host_port
            .rsplit_once(':')
            .map_or(host_port, |(host, _)| host),
    )
}

fn has_default_port(raw: &str, scheme: &str) -> bool {
    let Some(authority) = raw
        .split_once("://")
        .map(|(_, rest)| rest.split(['/', '?', '#']).next().unwrap_or(""))
    else {
        return false;
    };
    (scheme == "http" && authority.ends_with(":80"))
        || (scheme == "https" && authority.ends_with(":443"))
}

fn has_empty_path(raw: &str) -> bool {
    let Some(rest) = raw.split_once("://").map(|(_, rest)| rest) else {
        return false;
    };
    !rest.contains('/')
}

fn normalize_percent_hex(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut output = String::with_capacity(value.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && index + 2 < bytes.len()
            && bytes[index + 1].is_ascii_hexdigit()
            && bytes[index + 2].is_ascii_hexdigit()
        {
            output.push('%');
            output.push((bytes[index + 1] as char).to_ascii_uppercase());
            output.push((bytes[index + 2] as char).to_ascii_uppercase());
            index += 3;
        } else {
            output.push(bytes[index] as char);
            index += 1;
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FieldProvenance, OriginalUrl, ResultType};
    use std::collections::BTreeMap;

    fn normalized(raw: &str) -> NormalizedResult {
        NormalizedResult {
            schema_version: SCHEMA_VERSION.into(),
            title: None,
            raw_url: raw.into(),
            url: OriginalUrl(url::Url::parse(raw).unwrap()),
            provider: "p".into(),
            provider_rank: None,
            snippet: None,
            result_type: ResultType::Organic,
            published_at: None,
            author: None,
            language: None,
            file_type: None,
            thumbnail: None,
            metadata: BTreeMap::new(),
            provenance: BTreeMap::from([("url".into(), FieldProvenance::Reported)]),
            degradations: vec![],
        }
    }

    #[test]
    fn applies_only_safe_v1_transformations() {
        let result = canonicalize(normalized(
            "HTTP://EXAMPLE.COM:80/a%2f?utm_source=x&id=7&ref=y#section-2",
        ));
        assert_eq!(
            result.canonical_url.0.as_str(),
            "http://example.com/a%2F?id=7&ref=y#section-2"
        );
        assert!(result
            .transformations
            .contains(&CanonicalTransformation::RemoveDefaultPort));
        assert!(result
            .transformations
            .contains(&CanonicalTransformation::NormalizePercentHex));
        assert!(result.transformations.contains(
            &CanonicalTransformation::RemoveTrackingParameter("utm_source".into())
        ));
    }

    #[test]
    fn preserves_ambiguous_parameters_fragments_and_trailing_slash() {
        let with_slash = canonicalize(normalized("https://e.com/a/?ref=x&source=y#/route?tab=1"));
        let without_slash = canonicalize(normalized("https://e.com/a?ref=x&source=y#/route?tab=1"));
        assert_ne!(with_slash.canonical_url, without_slash.canonical_url);
        assert_eq!(with_slash.canonical_url.0.fragment(), Some("/route?tab=1"));
        assert_eq!(with_slash.canonical_url.0.query(), Some("ref=x&source=y"));
    }

    #[test]
    fn canonicalization_is_idempotent() {
        let first = canonicalize(normalized("https://EXAMPLE.com:443?utm_source=x&id=7#"));
        let second = canonicalize(normalized(first.canonical_url.0.as_str()));
        assert_eq!(first.canonical_url, second.canonical_url);
        assert!(second.transformations.is_empty());
    }

    #[test]
    fn malformed_percent_escape_is_preserved_and_degraded() {
        let result = canonicalize(normalized("https://example.com/a%zz"));
        assert_eq!(
            result.canonicalization_status,
            CanonicalizationStatus::Degraded
        );
        assert_eq!(result.canonical_url.0.as_str(), "https://example.com/a%zz");
        assert!(result
            .degradations
            .iter()
            .any(|value| value.code == "canonicalization_incomplete"));
    }
}
