//! Offline pricing table in USD per 1M tokens. Never fetched over the network.
//!
//! Defaults follow CLAUDE.md: cache reads bill at 10% of base input and cache
//! writes at 125%. Two refinements from Anthropic's published prices:
//! - Opus 5.5 and Fable 5.1 have cheaper cache reads than 10% (overridden below).
//! - Claude Code mostly writes 1-hour cache entries, which bill at 2x base input.
//!   `usage.cache_creation.ephemeral_1h_input_tokens` tells us how many.

use super::types::TokenCounts;

const CACHE_WRITE_5M: f64 = 1.25;
const CACHE_WRITE_1H: f64 = 2.0;
const CACHE_READ: f64 = 0.1;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModelRates {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    /// Applied to every rate when the response ran in fast mode (`usage.speed == "fast"`).
    pub fast_multiplier: f64,
    pub long_context: Option<LongContext>,
}

/// Higher rates for requests whose prompt exceeds `threshold` tokens.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LongContext {
    pub threshold: u64,
    pub input: f64,
    pub output: f64,
}

impl ModelRates {
    const fn new(input: f64, output: f64) -> Self {
        Self {
            input,
            output,
            cache_read: input * CACHE_READ,
            fast_multiplier: 1.0,
            long_context: None,
        }
    }

    const fn with_cache_read(mut self, price: f64) -> Self {
        self.cache_read = price;
        self
    }

    const fn with_fast(mut self, multiplier: f64) -> Self {
        self.fast_multiplier = multiplier;
        self
    }

    const fn with_long_context(mut self, threshold: u64, input: f64, output: f64) -> Self {
        self.long_context = Some(LongContext {
            threshold,
            input,
            output,
        });
        self
    }
}

const FABLE_5_1: ModelRates = ModelRates::new(10.0, 50.0).with_cache_read(0.25);
const OPUS_5_5: ModelRates = ModelRates::new(4.0, 20.0)
    .with_cache_read(0.20)
    .with_fast(2.0);
const SONNET_5_5: ModelRates = ModelRates::new(2.0, 10.0);
const HAIKU_5_5: ModelRates = ModelRates::new(0.10, 0.50).with_long_context(100_000, 0.50, 2.50);
const SONNET_4_X: ModelRates = ModelRates::new(3.0, 15.0).with_long_context(200_000, 6.0, 22.5);

/// Substring patterns, most specific first; the first match wins.
const MODEL_PRICES: &[(&str, ModelRates)] = &[
    ("fable-5-1", FABLE_5_1),
    ("mythos-5-1", FABLE_5_1),
    ("fable-5", ModelRates::new(10.0, 50.0)),
    ("mythos-5", ModelRates::new(10.0, 50.0)),
    ("opus-5-5", OPUS_5_5),
    ("opus-5", ModelRates::new(5.0, 25.0).with_fast(2.0)),
    ("opus-4-8", ModelRates::new(5.0, 25.0).with_fast(2.0)),
    ("opus-4-7", ModelRates::new(5.0, 25.0)),
    // Opus 4.6 fast mode was a research preview at 6x ($30 / $150).
    ("opus-4-6", ModelRates::new(5.0, 25.0).with_fast(6.0)),
    ("opus-4-5", ModelRates::new(5.0, 25.0)),
    ("opus-4-1", ModelRates::new(15.0, 75.0)),
    ("opus-4", ModelRates::new(15.0, 75.0)),
    ("3-opus", ModelRates::new(15.0, 75.0)),
    ("sonnet-5-5", SONNET_5_5),
    ("sonnet-5", ModelRates::new(2.0, 10.0)),
    ("sonnet-4-6", ModelRates::new(3.0, 15.0)),
    ("sonnet-4-5", SONNET_4_X),
    ("sonnet-4", SONNET_4_X),
    ("3-7-sonnet", ModelRates::new(3.0, 15.0)),
    ("3-5-sonnet", ModelRates::new(3.0, 15.0)),
    ("haiku-5-5", HAIKU_5_5),
    ("haiku-4-5", ModelRates::new(1.0, 5.0)),
    ("3-5-haiku", ModelRates::new(0.80, 4.0)),
    ("3-haiku", ModelRates::new(0.25, 1.25)),
];

/// Versions missing from the table are priced as the newest model in their family.
const FAMILY_FALLBACKS: &[(&str, ModelRates)] = &[
    ("fable", FABLE_5_1),
    ("mythos", FABLE_5_1),
    ("opus", OPUS_5_5),
    ("sonnet", SONNET_5_5),
    ("haiku", HAIKU_5_5),
];

/// Looks up rates for a model id such as `claude-opus-4-5-20251101`,
/// `anthropic.claude-sonnet-4-5-v1:0` or `claude-opus-4-5@20251101`.
pub fn rates_for(model: &str) -> Option<ModelRates> {
    let id = model.to_ascii_lowercase().replace(['.', '_'], "-");
    MODEL_PRICES
        .iter()
        .chain(FAMILY_FALLBACKS)
        .find(|(pattern, _)| id.contains(pattern))
        .map(|&(_, rates)| rates)
}

/// API-equivalent cost of one response. `cache_write_1h` is the part of
/// `tokens.cache_creation` written with the 1-hour TTL.
pub fn cost_usd(rates: &ModelRates, tokens: &TokenCounts, cache_write_1h: u64, fast: bool) -> f64 {
    let (input, output, scale) = match rates.long_context {
        Some(lc) if tokens.prompt() > lc.threshold => (lc.input, lc.output, lc.input / rates.input),
        _ => (rates.input, rates.output, 1.0),
    };
    let write_1h = cache_write_1h.min(tokens.cache_creation);
    let write_5m = tokens.cache_creation - write_1h;
    let speed = if fast { rates.fast_multiplier } else { 1.0 };

    let usd = tokens.input as f64 * input
        + tokens.output as f64 * output
        + write_5m as f64 * input * CACHE_WRITE_5M
        + write_1h as f64 * input * CACHE_WRITE_1H
        + tokens.cache_read as f64 * rates.cache_read * scale;
    usd * speed / 1_000_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input_price(model: &str) -> f64 {
        rates_for(model).unwrap().input
    }

    #[test]
    fn matches_most_specific_version_first() {
        assert_eq!(input_price("claude-opus-5-5"), 4.0);
        assert_eq!(input_price("claude-opus-5"), 5.0);
        assert_eq!(input_price("claude-opus-4-5-20251101"), 5.0);
        assert_eq!(input_price("claude-opus-4-20250514"), 15.0);
        assert_eq!(input_price("claude-opus-4-1-20250805"), 15.0);
        assert_eq!(input_price("claude-sonnet-4-6"), 3.0);
        assert_eq!(input_price("claude-sonnet-4-5-20250929"), 3.0);
        assert_eq!(input_price("claude-haiku-4-5-20251001"), 1.0);
        assert_eq!(input_price("claude-3-5-haiku-20241022"), 0.8);
        assert_eq!(input_price("claude-fable-5-1"), 10.0);
    }

    #[test]
    fn normalizes_provider_ids_and_falls_back_by_family() {
        assert_eq!(input_price("anthropic.claude-opus-4-5-20251101-v1:0"), 5.0);
        assert_eq!(input_price("claude-opus-4-5@20251101"), 5.0);
        assert_eq!(input_price("claude-sonnet-9-9"), 2.0);
        assert_eq!(rates_for("<synthetic>"), None);
    }

    #[test]
    fn prices_cache_tiers_separately() {
        let rates = rates_for("claude-opus-4-5").unwrap(); // $5 / $25
        let tokens = TokenCounts {
            input: 1_000_000,
            output: 1_000_000,
            cache_creation: 2_000_000,
            cache_read: 1_000_000,
        };
        // 5 + 25 + (1M * 5 * 1.25) + (1M * 5 * 2) + (1M * 0.5)
        let cost = cost_usd(&rates, &tokens, 1_000_000, false);
        assert!((cost - (5.0 + 25.0 + 6.25 + 10.0 + 0.5)).abs() < 1e-9);
        assert!((cost_usd(&rates_for("claude-opus-5-5").unwrap(), &tokens, 0, true)
            - 2.0 * (4.0 + 20.0 + 10.0 + 0.2))
            .abs()
            < 1e-9);
    }

    #[test]
    fn cache_reads_bill_a_tenth_of_input_unless_overridden() {
        let read = TokenCounts { cache_read: 1_000_000, ..Default::default() };
        for (model, price) in [
            ("claude-sonnet-5-5", 0.20),
            ("claude-sonnet-9-9", 0.20), // family fallback
            ("claude-opus-4-6", 0.50),
            ("claude-opus-5-5", 0.20),
            ("claude-fable-5-1", 0.25),
        ] {
            let cost = cost_usd(&rates_for(model).unwrap(), &read, 0, false);
            assert!((cost - price).abs() < 1e-9, "{model}: {cost}");
        }
    }

    #[test]
    fn applies_long_context_rates_above_threshold() {
        let rates = rates_for("claude-haiku-5-5").unwrap();
        let short = TokenCounts { input: 100_000, ..Default::default() };
        let long = TokenCounts { input: 100_001, ..Default::default() };
        assert!((cost_usd(&rates, &short, 0, false) - 0.01).abs() < 1e-9);
        assert!((cost_usd(&rates, &long, 0, false) - 0.0500005).abs() < 1e-9);
    }
}
