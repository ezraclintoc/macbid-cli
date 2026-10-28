//! Client-side filters.
//!
//! Everything Mac.Bid can filter on (category, location, price, condition,
//! closing date, flags) is expressed server-side in [`SearchFilter`]. The two
//! things its index has no concept of are a keyword blocklist and "the title
//! itself must contain the query" — those are applied here, after the fetch.

use crate::models::Lot;

/// Whether text comparisons are case-sensitive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CaseMode {
    /// Fold case (default, matches how the site searches).
    #[default]
    IgnoreCase,
    /// Require the exact capitalization the user typed.
    Exact,
}

/// Post-fetch text filters.
#[derive(Debug, Clone, Default)]
pub struct LocalFilter {
    /// Drop lots whose title contains any of these terms.
    pub exclude_text: Vec<String>,
    /// Keep only lots whose title contains every one of these words
    /// (`--titles-only`, and implied by `--case-sensitive`).
    pub require_words: Vec<String>,
    /// Case handling for both lists.
    pub case: CaseMode,
}

impl LocalFilter {
    /// True when there is nothing to do.
    pub fn is_empty(&self) -> bool {
        self.exclude_text.is_empty() && self.require_words.is_empty()
    }

    /// Apply to a batch of lots. Returns `(kept, dropped_count)`.
    pub fn apply(&self, lots: Vec<Lot>) -> (Vec<Lot>, usize) {
        if self.is_empty() {
            return (lots, 0);
        }
        let before = lots.len();
        let kept: Vec<Lot> = lots.into_iter().filter(|lot| self.keeps(lot)).collect();
        let dropped = before - kept.len();
        (kept, dropped)
    }

    /// Whether one lot survives the local filters.
    pub fn keeps(&self, lot: &Lot) -> bool {
        let title = self.case.apply(lot.title());
        if self
            .exclude_text
            .iter()
            .any(|term| title.contains(&self.case.apply(term)))
        {
            return false;
        }
        self.require_words
            .iter()
            .all(|word| title.contains(&self.case.apply(word)))
    }

    /// Fold a string according to the case mode (used by the CLI for local
    /// comparisons it does itself).
    pub fn normalize(&self, s: &str) -> String {
        self.case.apply(s)
    }
}

impl CaseMode {
    /// Normalize one string for comparison.
    pub fn apply(self, s: &str) -> String {
        match self {
            Self::IgnoreCase => s.to_lowercase(),
            Self::Exact => s.to_string(),
        }
    }
}

impl Lot {
    /// The text the local filters look at.
    pub fn title(&self) -> &str {
        if !self.product_name.is_empty() {
            &self.product_name
        } else {
            &self.auction_title
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lot(title: &str) -> Lot {
        serde_json::from_value(serde_json::json!({ "product_name": title })).unwrap()
    }

    #[test]
    fn empty_filter_keeps_everything() {
        let (kept, dropped) = LocalFilter::default().apply(vec![lot("a"), lot("b")]);
        assert_eq!(kept.len(), 2);
        assert_eq!(dropped, 0);
    }

    #[test]
    fn exclude_text_is_case_insensitive_by_default() {
        let f = LocalFilter {
            exclude_text: vec!["kids".into()],
            ..Default::default()
        };
        let (kept, dropped) = f.apply(vec![lot("Kids Bike"), lot("Adult Bike")]);
        assert_eq!(kept.len(), 1);
        assert_eq!(dropped, 1);
        assert_eq!(kept[0].title(), "Adult Bike");
    }

    #[test]
    fn case_sensitive_exclude_needs_the_exact_spelling() {
        let f = LocalFilter {
            exclude_text: vec!["Kids".into()],
            case: CaseMode::Exact,
            ..Default::default()
        };
        let (kept, _) = f.apply(vec![lot("kids bike"), lot("Kids bike")]);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].title(), "kids bike");
    }

    #[test]
    fn require_words_needs_every_word_in_the_title() {
        let f = LocalFilter {
            require_words: vec!["lg".into(), "oled".into()],
            ..Default::default()
        };
        let kept = f.apply(vec![
            lot("LG OLED TV"),
            lot("LG LED TV"),
            lot("Sony OLED TV"),
        ]);
        assert_eq!(kept.0.len(), 1);
        assert_eq!(kept.0[0].title(), "LG OLED TV");
    }
}
