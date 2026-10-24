//! OpenAI's API prices for Codex's models in USD per 1M tokens: the Standard
//! tier of developers.openai.com/api/docs/pricing as of 2026-10-21. Never
//! fetched over the network.
//!
//! Each response is priced on its own: uncached input, cached input, cache
//! writes and output (reasoning is part of it) at its model's rates, all of
//! them at the long-context rates once its input exceeds 272K tokens. A
//! model without a cache-write price bills writes as uncached input. Codex
//! logs no service tier, so Fast-tier turns (2×) price as Standard.
//!
//! The local `pricing.json` that reprices Claude's models (`usage::Pricing`)
//! adds or replaces these prices too: `cacheRead` is the cached-input price
//! and `cacheWrite5m` the cache-write one, defaulting to 10% and 125% of
//! input as for Claude (`cacheWrite1h` and `fastMultiplier` do not apply).
//! An override has no long-context tier. Models in neither table stay
//! unpriced rather than guessed.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use super::types::{CodexCostSplit, CodexTokens};
use crate::usage::Pricing;

/// A response with more input tokens than this bills at long-context rates.
const LONG_CONTEXT_INPUT: u64 = 272_000;

/// Words that make another model of a name, priced apart: `gpt-5.1` does
/// not name `gpt-5.1-codex-mini`, nor `gpt-5` `gpt-5-pro`.
const VARIANTS: [&str; 3] = ["mini", "nano", "pro"];

/// A model name, its standard rates and its long-context rates.
type ModelPrice = (&'static str, [f64; 4], Option<[f64; 4]>);

/// Per model as OpenAI names it: uncached input, cached input, cache write
/// (30-minute) and output, then the same for a response with more than 272K
/// input tokens. A model takes the most specific name that names it (`rank`).
const MODEL_PRICES: &[ModelPrice] = &[
    (
        "gpt-6-astra",
        [10.0, 1.0, 12.5, 50.0],
        Some([20.0, 2.0, 25.0, 75.0]),
    ),
    (
        "gpt-6.1-sol",
        [2.0, 0.10, 2.5, 10.0],
        Some([4.0, 0.20, 5.0, 15.0]),
    ),
    (
        "gpt-6-sol",
        [2.0, 0.20, 2.5, 10.0],
        Some([4.0, 0.40, 5.0, 15.0]),
    ),
    (
        "gpt-6-luna",
        [0.10, 0.01, 0.125, 0.50],
        Some([0.20, 0.02, 0.25, 0.75]),
    ),
    // A promotional price, at least through 2026-11-21.
    (
        "gpt-5.6-sol",
        [4.0, 0.40, 5.0, 20.0],
        Some([8.0, 0.80, 10.0, 30.0]),
    ),
    (
        "gpt-5.6-terra",
        [2.0, 0.20, 2.5, 12.0],
        Some([4.0, 0.40, 5.0, 18.0]),
    ),
    (
        "gpt-5.6-luna",
        [0.20, 0.02, 0.25, 1.20],
        Some([0.40, 0.04, 0.50, 1.80]),
    ),
    // No cache-write price from here on: writes bill as input.
    (
        "gpt-5.5",
        [5.0, 0.50, 5.0, 30.0],
        Some([10.0, 1.0, 10.0, 45.0]),
    ),
    (
        "gpt-5.4",
        [2.50, 0.25, 2.50, 15.0],
        Some([5.0, 0.50, 5.0, 22.50]),
    ),
    ("gpt-5.4-mini", [0.75, 0.075, 0.75, 4.50], None),
    ("gpt-5.3-codex", [1.75, 0.175, 1.75, 14.0], None),
    ("gpt-5.2", [1.75, 0.175, 1.75, 14.0], None),
    ("gpt-5.1", [1.25, 0.125, 1.25, 10.0], None),
    ("gpt-5", [1.25, 0.125, 1.25, 10.0], None),
    ("gpt-5-mini", [0.25, 0.025, 0.25, 2.0], None),
];

/// USD per 1M tokens.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rates {
    pub input: f64,
    pub cached: f64,
    pub cache_write: f64,
    pub output: f64,
}

impl From<[f64; 4]> for Rates {
    fn from([input, cached, cache_write, output]: [f64; 4]) -> Self {
        Self {
            input,
            cached,
            cache_write,
            output,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModelRates {
    pub standard: Rates,
    /// For a response with more than 272K input tokens.
    pub long_context: Option<Rates>,
}

impl ModelRates {
    /// What one response would cost, USD, by kind of token. Codex's `input`
    /// holds the cached tokens (as Codex's own totals confirm) and is taken
    /// to hold the cache writes as well, which the logs cannot tell: every
    /// write in them so far is zero. So uncached input is
    /// `input − cached − cache_write`. Reasoning is output, at its price.
    pub fn cost_split(&self, tokens: &CodexTokens) -> CodexCostSplit {
        let rates = match self.long_context {
            Some(long) if tokens.input > LONG_CONTEXT_INPUT => long,
            _ => self.standard,
        };
        let uncached = tokens
            .input
            .saturating_sub(tokens.cached)
            .saturating_sub(tokens.cache_write);
        let usd = |count: u64, rate: f64| count as f64 * rate / 1_000_000.0;
        let cached = usd(tokens.cached, rates.cached);
        let cache_write = usd(tokens.cache_write, rates.cache_write);
        CodexCostSplit {
            input: usd(uncached, rates.input) + cached + cache_write,
            cached,
            cache_write,
            output: usd(tokens.output, rates.output),
            reasoning: usd(tokens.reasoning.min(tokens.output), rates.output),
        }
    }
}

/// Lower case with `-` for `.` and `_`, the way `usage::Pricing` keys its
/// overrides: `GPT-5.1` is `gpt-5-1`.
fn normalize(model: &str) -> String {
    model.trim().to_ascii_lowercase().replace(['.', '_'], "-")
}

/// How specifically `key` names the model `id` (both normalized): where the
/// match ends in `id`, then the key's length. A key names a model only as a
/// whole: `gpt-5` names `gpt-5-codex` and `gpt-5-2025-08-07`, but not
/// `gpt-5.1` (`gpt-5-1`), `gpt-50` or a variant such as `gpt-5-mini`.
fn rank(id: &str, key: &str) -> Option<(usize, usize)> {
    let start = id.find(key)?;
    let end = start + key.len();
    let glued = |c: Option<char>| c.is_some_and(|c| c.is_ascii_alphanumeric());
    if glued(id[..start].chars().next_back()) || glued(id[end..].chars().next()) {
        return None;
    }
    let words: Vec<&str> = id[end..]
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect();
    // A minor version (`gpt-5` then `1`), not a date (`2025`).
    let version = words
        .first()
        .is_some_and(|word| word.len() <= 2 && word.bytes().all(|b| b.is_ascii_digit()));
    if version || words.iter().any(|word| VARIANTS.contains(word)) {
        return None;
    }
    Some((end, key.len()))
}

/// A model's rates: those of the most specific name among the built-in ones
/// and the `pricing.json` overrides, an override winning a tie, as for
/// Claude. With the override's index in `Pricing::overrides` when one won.
pub fn rates_for(pricing: &Pricing, model: &str) -> Option<(ModelRates, Option<usize>)> {
    let id = normalize(model);
    let builtin = MODEL_PRICES
        .iter()
        .filter_map(|&(name, standard, long_context)| {
            let rates = ModelRates {
                standard: standard.into(),
                long_context: long_context.map(Rates::from),
            };
            Some((rank(&id, &normalize(name))?, rates))
        })
        .max_by_key(|(rank, _)| *rank);
    let custom = pricing
        .overrides()
        .enumerate()
        .filter_map(|(index, (key, rates))| {
            let rates = ModelRates {
                standard: Rates {
                    input: rates.input,
                    cached: rates.cache_read,
                    cache_write: rates.cache_write_5m,
                    output: rates.output,
                },
                long_context: None,
            };
            Some((rank(&id, key)?, (rates, index)))
        })
        // Of equally specific names, the first in the file wins.
        .reduce(|best, next| if next.0 > best.0 { next } else { best });
    match (custom, builtin) {
        (Some(custom), Some(builtin)) if custom.0 < builtin.0 => Some((builtin.1, None)),
        (Some((_, (rates, index))), _) => Some((rates, Some(index))),
        (None, builtin) => builtin.map(|(_, rates)| (rates, None)),
    }
}

/// Prices responses by their model, looking each model up once.
pub struct Prices<'p> {
    pricing: &'p Pricing,
    /// Per model: its rates, and the override they come from, if priced.
    models: HashMap<Arc<str>, Option<(ModelRates, Option<usize>)>>,
}

impl<'p> Prices<'p> {
    pub fn new(pricing: &'p Pricing) -> Self {
        Self {
            pricing,
            models: HashMap::new(),
        }
    }

    /// What a response of `model` would cost, USD, by kind of token; `None`
    /// when the model is unknown or has no price.
    pub fn cost(
        &mut self,
        model: Option<&Arc<str>>,
        tokens: &CodexTokens,
    ) -> Option<CodexCostSplit> {
        let model = model?;
        let pricing = self.pricing;
        let (rates, _) = self
            .models
            .entry(Arc::clone(model))
            .or_insert_with(|| rates_for(pricing, model))
            .as_ref()?;
        Some(rates.cost_split(tokens))
    }

    /// Entries of `pricing.json` that priced a model so far.
    pub fn custom_count(&self) -> usize {
        self.models
            .values()
            .filter_map(|found| found.as_ref()?.1)
            .collect::<HashSet<_>>()
            .len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(input: u64, cached: u64, cache_write: u64, output: u64) -> CodexTokens {
        CodexTokens {
            input,
            cached,
            cache_write,
            output,
            reasoning: 0,
        }
    }

    /// Built-in Standard-tier rates: input, cached input, cache write, output.
    fn standard(model: &str) -> Option<[f64; 4]> {
        let (rates, custom) = rates_for(&Pricing::default(), model)?;
        assert_eq!(custom, None);
        let Rates {
            input,
            cached,
            cache_write,
            output,
        } = rates.standard;
        Some([input, cached, cache_write, output])
    }

    fn cost(pricing: &Pricing, model: &str, tokens: CodexTokens) -> f64 {
        rates_for(pricing, model)
            .unwrap()
            .0
            .cost_split(&tokens)
            .total()
    }

    #[test]
    fn prices_every_model_in_the_table() {
        for (model, rates) in [
            ("gpt-6-astra", [10.0, 1.0, 12.5, 50.0]),
            ("gpt-6.1-sol", [2.0, 0.10, 2.5, 10.0]),
            ("gpt-6-sol", [2.0, 0.20, 2.5, 10.0]),
            ("gpt-6-luna", [0.10, 0.01, 0.125, 0.50]),
            ("gpt-5.6-sol", [4.0, 0.40, 5.0, 20.0]),
            ("gpt-5.6-terra", [2.0, 0.20, 2.5, 12.0]),
            ("gpt-5.6-luna", [0.20, 0.02, 0.25, 1.20]),
            // No cache-write price: writes bill as input.
            ("gpt-5.5", [5.0, 0.50, 5.0, 30.0]),
            ("gpt-5.4", [2.50, 0.25, 2.50, 15.0]),
            ("gpt-5.4-mini", [0.75, 0.075, 0.75, 4.50]),
            ("gpt-5.3-codex", [1.75, 0.175, 1.75, 14.0]),
            ("gpt-5.2", [1.75, 0.175, 1.75, 14.0]),
            ("gpt-5.1", [1.25, 0.125, 1.25, 10.0]),
            ("gpt-5", [1.25, 0.125, 1.25, 10.0]),
            ("gpt-5-mini", [0.25, 0.025, 0.25, 2.0]),
            // Spelled otherwise, dated, or a Codex flavour of a listed model.
            (" GPT-6.1-Sol ", [2.0, 0.10, 2.5, 10.0]),
            ("gpt_6_astra", [10.0, 1.0, 12.5, 50.0]),
            ("gpt-5-2025-08-07", [1.25, 0.125, 1.25, 10.0]),
            ("gpt-5-codex", [1.25, 0.125, 1.25, 10.0]),
            ("gpt-5.1-codex-max", [1.25, 0.125, 1.25, 10.0]),
            ("gpt-5.2-codex", [1.75, 0.175, 1.75, 14.0]),
            ("gpt-5-mini-2025-08-07", [0.25, 0.025, 0.25, 2.0]),
        ] {
            assert_eq!(standard(model), Some(rates), "{model}");
        }
    }

    #[test]
    fn variants_and_versions_missing_from_the_table_stay_unpriced() {
        for model in [
            // A mini (nano, pro) variant is not its base model.
            "gpt-5.1-codex-mini",
            "gpt-5-nano",
            "gpt-5.5-pro",
            "gpt-6-astra-mini",
            // Nor is a missing version the nearest one listed.
            "gpt-5.7-sol",
            "gpt-6.2-sol",
            "gpt-6",
            "gpt-50",
            "chatgpt-5",
            // Models OpenAI does not list.
            "gpt-reserve",
            "codex-auto-review",
        ] {
            assert_eq!(standard(model), None, "{model}");
        }
    }

    #[test]
    fn a_response_over_272k_input_bills_all_of_it_at_long_context_rates() {
        let pricing = Pricing::default();
        assert!((cost(&pricing, "gpt-6-astra", tokens(272_000, 0, 0, 0)) - 2.72).abs() < 1e-9);
        // One token more: $20 for every input token.
        let over = cost(&pricing, "gpt-6-astra", tokens(272_001, 0, 0, 0));
        assert!((over - 5.440_02).abs() < 1e-9);
        // 50K uncached × $20, 250K cached × $2, 10K output × $75.
        let mixed = cost(&pricing, "gpt-6-astra", tokens(300_000, 250_000, 0, 10_000));
        assert!((mixed - (1.0 + 0.5 + 0.75)).abs() < 1e-9);
        let sol = cost(&pricing, "gpt-6.1-sol", tokens(300_000, 250_000, 0, 10_000));
        assert!((sol - (0.2 + 0.05 + 0.15)).abs() < 1e-9);
        // Without a long-context tier, the Standard rates hold at any size.
        let mini = cost(&pricing, "gpt-5.4-mini", tokens(1_000_000, 0, 0, 0));
        assert!((mini - 0.75).abs() < 1e-9);
    }

    #[test]
    fn cache_writes_come_out_of_input_at_their_own_price() {
        let pricing = Pricing::default();
        // 200K input: 100K cached, 50K written to the cache, 50K neither; 10K output.
        let used = tokens(200_000, 100_000, 50_000, 10_000);
        // 50K × $10 + 100K × $1 + 50K × $12.50 + 10K × $50.
        let astra = cost(&pricing, "gpt-6-astra", used);
        assert!((astra - (0.5 + 0.1 + 0.625 + 0.5)).abs() < 1e-9);
        // gpt-5.5 has no write price: its writes bill as input, $5.
        let older = cost(&pricing, "gpt-5.5", used);
        assert!((older - (0.25 + 0.05 + 0.25 + 0.3)).abs() < 1e-9);
        // More cached and written than input leaves no uncached input, not less.
        let odd = cost(&pricing, "gpt-6-astra", tokens(100, 80, 40, 0));
        assert!((odd - (80.0 * 1.0 + 40.0 * 12.5) / 1e6).abs() < 1e-12);
    }

    #[test]
    fn reasoning_bills_as_the_output_it_is_part_of() {
        let pricing = Pricing::default();
        let plain = tokens(1_000, 0, 0, 2_000);
        let reasoned = CodexTokens {
            reasoning: 1_500,
            ..plain
        };
        let cost = |used| cost(&pricing, "gpt-6-astra", used);
        assert_eq!(cost(plain), cost(reasoned));
        assert!((cost(plain) - (0.01 + 0.1)).abs() < 1e-12);
    }

    #[test]
    fn a_cost_splits_by_kind_of_token_as_the_tokens_do() {
        let (rates, _) = rates_for(&Pricing::default(), "gpt-6-astra").unwrap();
        // 200K input: 100K cached, 50K written, 50K neither; 10K output, 4K of it reasoning.
        let used = CodexTokens {
            reasoning: 4_000,
            ..tokens(200_000, 100_000, 50_000, 10_000)
        };
        let split = rates.cost_split(&used);
        let close = |a: f64, b: f64| (a - b).abs() < 1e-12;
        assert!(close(split.cached, 0.1), "100K × $1");
        assert!(close(split.cache_write, 0.625), "50K × $12.50");
        assert!(
            close(split.input, 0.5 + 0.1 + 0.625),
            "uncached, cached and written"
        );
        assert!(close(split.output, 0.5), "10K × $50");
        assert!(close(split.reasoning, 0.2), "4K × $50, within the output");
        assert!(close(split.total(), 0.5 + 0.1 + 0.625 + 0.5));
    }

    #[test]
    fn local_overrides_add_and_replace_codex_prices() {
        let pricing = Pricing::parse(
            r#"{
                "claude-opus-6": { "input": 6, "output": 30 },
                "GPT-6.2-Sol": { "input": 3, "output": 15 },
                "gpt-6-astra": { "input": 5, "output": 25, "cacheRead": 0.25, "cacheWrite5m": 6 },
                "gpt-5": { "input": 1, "output": 8 }
            }"#,
        )
        .unwrap();
        let found = |model: &str| rates_for(&pricing, model);
        // A new model, its cache prices 10% and 125% of input as for Claude.
        let (sol, custom) = found("gpt-6.2-sol").unwrap();
        assert!(custom.is_some());
        let Rates {
            input,
            cached,
            cache_write,
            output,
        } = sol.standard;
        assert_eq!((input, cache_write, output), (3.0, 3.75, 15.0));
        assert!((cached - 0.3).abs() < 1e-12);
        assert_eq!(sol.long_context, None);
        // A replaced one, with no long-context tier any more.
        let used = tokens(300_000, 0, 0, 0);
        assert!((cost(&pricing, "gpt-6-astra", used) - 1.5).abs() < 1e-9);
        let (astra, _) = found("gpt-6-astra").unwrap();
        assert_eq!(astra.standard, Rates::from([5.0, 0.25, 6.0, 25.0]));
        // `gpt-5` reprices what it names, not what a built-in name names better.
        for model in ["gpt-5", "gpt-5-codex", "gpt-5-2025-08-07"] {
            assert_eq!(found(model).unwrap().0.standard.input, 1.0, "{model}");
        }
        for model in ["gpt-5.1", "gpt-5-mini", "gpt-6.1-sol"] {
            assert_eq!(
                found(model),
                rates_for(&Pricing::default(), model),
                "{model}"
            );
        }
        assert_eq!(found("gpt-5.7-sol"), None);
    }

    #[test]
    fn prices_count_the_overrides_they_priced_with() {
        let pricing = Pricing::parse(
            r#"{
                "claude-opus-6": { "input": 6, "output": 30 },
                "gpt-5": { "input": 1, "output": 8 },
                "gpt-6-astra": { "input": 5, "output": 25 },
                "gpt-9": { "input": 1, "output": 1 }
            }"#,
        )
        .unwrap();
        let mut prices = Prices::new(&pricing);
        let used = tokens(100_000, 0, 0, 0);
        let mut price = |model: Option<&str>| {
            prices
                .cost(model.map(Arc::from).as_ref(), &used)
                .map(|cost| cost.total())
        };
        assert!((price(Some("gpt-6-astra")).unwrap() - 0.5).abs() < 1e-12);
        assert!((price(Some("gpt-5-codex")).unwrap() - 0.1).abs() < 1e-12);
        assert!((price(Some("gpt-5-2025-08-07")).unwrap() - 0.1).abs() < 1e-12);
        assert!((price(Some("gpt-6.1-sol")).unwrap() - 0.2).abs() < 1e-12);
        assert_eq!(price(Some("gpt-reserve")), None);
        assert_eq!(price(None), None);
        assert_eq!(prices.custom_count(), 2, "gpt-6-astra and gpt-5");
    }
}
