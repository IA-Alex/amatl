use std::collections::BTreeSet;
use unicode_normalization::UnicodeNormalization;

pub(crate) fn normalized_text(value: &str) -> String {
    let mut casefolded = String::new();
    for character in value.nfkc() {
        match character {
            'ß' | 'ẞ' => casefolded.push_str("ss"),
            'ς' => casefolded.push('σ'),
            _ => casefolded.extend(character.to_lowercase()),
        }
    }
    casefolded.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub(crate) fn tokens(value: &str) -> BTreeSet<String> {
    normalized_text(value)
        .split(|character: char| !character.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nfkc_casefold_is_stable_for_compatibility_and_special_case_letters() {
        assert_eq!(
            normalized_text("ＳＴＲＡＳＳＥ"),
            normalized_text("strasse")
        );
        assert_eq!(normalized_text("Straße"), normalized_text("STRASSE"));
        assert_eq!(normalized_text("ΟΣ"), normalized_text("ος"));
    }
}
