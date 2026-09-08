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

/// Like [`normalized_text`] but additionally folds common Unicode
/// confusables/homoglyphs to their ASCII lookalikes, so that multilingual
/// strings that differ only in script (e.g. Cyrillic `а` vs Latin `a`, Greek
/// `ο` vs Latin `o`) compare equal. Used by deduplication title similarity.
pub(crate) fn normalized_confusable_text(value: &str) -> String {
    let mut casefolded = String::new();
    for character in value.nfkc() {
        match character {
            'ß' | 'ẞ' => casefolded.push_str("ss"),
            'ς' => casefolded.push('σ'),
            _ => match fold_confusable_char(character) {
                Some(folded) => casefolded.push_str(folded),
                None => casefolded.extend(character.to_lowercase()),
            },
        }
    }
    casefolded.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Map a single confusable character to its canonical ASCII lookalike, if any.
fn fold_confusable_char(character: char) -> Option<&'static str> {
    Some(match character {
        // Cyrillic lookalikes.
        'а' | 'А' => "a",
        'е' | 'Е' => "e",
        'о' | 'О' => "o",
        'р' | 'Р' => "p",
        'с' | 'С' => "c",
        'у' | 'У' => "y",
        'х' | 'Х' => "x",
        'і' | 'І' => "i",
        'ј' | 'Ј' => "j",
        'ѕ' | 'Ѕ' => "s",
        'ѵ' | 'Ѵ' => "v",
        'ѡ' | 'Ѡ' => "w",
        'һ' | 'Һ' => "h",
        'ԁ' | 'Ԁ' => "d",
        'ԛ' | 'Ԛ' => "q",
        'т' | 'Т' => "t",
        // Soft sign only. The hard sign `ъ` is a different letter that would
        // fold to the same `b`, making `ьx` and `ъx` compare equal.
        'Ь' | 'ь' => "b",
        // Greek lookalikes.
        //
        // Only characters that are *visually* confusable with a Latin letter
        // belong here. Phonetic transliteration does not: mapping θ and φ to
        // `o`, or γ and ψ to `y`, collapses letters that look nothing alike and
        // makes genuinely different Greek titles compare equal in
        // `dedupe::title_similarity`.
        //
        // Case is split deliberately, because folding runs before lowercasing:
        // Η (capital eta) is a homoglyph of `h`, not of `n`, and Ρ (capital
        // rho) of `p` while ρ (small rho) is also `p`.
        'Α' => "a",
        'Β' => "b",
        'Ε' => "e",
        'Ζ' => "z",
        'Η' => "h",
        'Ι' => "i",
        'Κ' => "k",
        'Μ' => "m",
        'Ν' => "n",
        'Ο' => "o",
        'Ρ' => "p",
        'Τ' => "t",
        'Υ' => "y",
        'Χ' => "x",
        'α' => "a",
        'ε' => "e",
        'ι' => "i",
        'κ' => "k",
        'ν' => "v",
        'ο' => "o",
        'ρ' => "p",
        'τ' => "t",
        'υ' => "u",
        'χ' => "x",
        // Latin extended lookalikes.
        'ɑ' | 'ɐ' | 'ɒ' | 'ᴀ' | 'ᴁ' => "a",
        'ʙ' => "b",
        'ᴄ' | 'ƈ' | 'ɕ' | 'ʗ' => "c",
        'ᴅ' => "d",
        'ᴇ' | 'ɛ' | 'ɜ' => "e",
        'ꜰ' => "f",
        'ɢ' => "g",
        'ʜ' => "h",
        'ɪ' | 'ɩ' => "i",
        'ᴊ' => "j",
        'ᴋ' => "k",
        'ʟ' | 'ⅼ' | 'ǀ' | 'ǁ' => "l",
        'ᴍ' => "m",
        'ɴ' => "n",
        'ᴏ' => "o",
        'ᴘ' => "p",
        'ǫ' => "q",
        'ʀ' => "r",
        'ꜱ' => "s",
        'ᴛ' => "t",
        'ᴜ' => "u",
        'ᴠ' => "v",
        'ᴡ' => "w",
        'ʏ' => "y",
        'ᴢ' | 'ʐ' => "z",
        // Roman numerals.
        'ⅰ' | 'Ⅰ' => "i",
        'ⅱ' | 'Ⅱ' => "ii",
        'ⅲ' | 'Ⅲ' => "iii",
        'ⅳ' | 'Ⅳ' => "iv",
        'ⅴ' | 'Ⅴ' => "v",
        'ⅵ' | 'Ⅵ' => "vi",
        'ⅶ' | 'Ⅶ' => "vii",
        'ⅷ' | 'Ⅷ' => "viii",
        'ⅸ' | 'Ⅸ' => "ix",
        'ⅹ' | 'Ⅹ' => "x",
        'ⅺ' | 'Ⅺ' => "xi",
        'ⅻ' | 'Ⅻ' => "xii",
        'ⅽ' | 'Ⅽ' => "c",
        'ⅾ' | 'Ⅾ' => "d",
        'ⅿ' | 'Ⅿ' => "m",
        // Superscript lookalikes.
        'ˡ' => "l",
        'ʰ' => "h",
        'ʸ' => "y",
        'ᵗ' => "t",
        'ᵘ' => "u",
        'ᵛ' => "v",
        'ᵃ' => "a",
        'ᵇ' => "b",
        'ᶜ' => "c",
        'ᵈ' => "d",
        'ᵉ' => "e",
        'ᶠ' => "f",
        'ᵍ' => "g",
        'ᵢ' => "i",
        'ʲ' => "j",
        'ᵏ' => "k",
        'ᵐ' => "m",
        'ⁿ' => "n",
        'ᵒ' => "o",
        'ᵖ' => "p",
        'ʳ' => "r",
        'ˢ' => "s",
        'ʷ' => "w",
        'ˣ' => "x",
        'ᶻ' => "z",
        'ǃ' => "i",
        _ => return None,
    })
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

    #[test]
    fn confusable_folding_collapses_homoglyph_scripts() {
        // Cyrillic vs Latin.
        assert_eq!(
            normalized_confusable_text("асинхронный"),
            normalized_confusable_text("acинхронный")
        );
        // Greek omicron vs Latin o.
        assert_eq!(
            normalized_confusable_text("προγραμματισμος"),
            normalized_confusable_text("προγραμματισμoς")
        );
        // Fullwidth handled by NFKC, then confusable folding.
        assert_eq!(
            normalized_confusable_text("ＳＴＲＡＳＳＥ"),
            normalized_confusable_text("strasse")
        );
    }

    #[test]
    fn confusable_folding_keeps_the_final_sigma_invariant() {
        // The final and medial forms of the same Greek word must agree. They
        // did not while `ς` was pushed straight to the output and `σ` was
        // separately folded to `s`.
        assert_eq!(
            normalized_confusable_text("ΟΣ"),
            normalized_confusable_text("ος")
        );
        assert_eq!(
            normalized_confusable_text("λόγος"),
            normalized_confusable_text("λόγοσ")
        );
        // And the same invariant `normalized_text` already guaranteed.
        assert_eq!(
            normalized_confusable_text("ΛΟΓΟΣ"),
            normalized_confusable_text("λογος")
        );
    }

    #[test]
    fn confusable_folding_keeps_visually_distinct_greek_letters_apart() {
        // Phonetic transliteration would collapse all of these onto the same
        // Latin letters (θ/φ→o, γ/ψ→y, π/ρ→p, χ/ξ→x).
        for (left, right) in [("θα", "φα"), ("γα", "ψα"), ("πα", "ρα"), ("ξα", "χα")]
        {
            assert_ne!(
                normalized_confusable_text(left),
                normalized_confusable_text(right),
                "{left} and {right} must not fold together"
            );
        }
    }
}
