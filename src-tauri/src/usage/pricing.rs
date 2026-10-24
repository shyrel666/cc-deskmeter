//! Offline pricing table in USD per 1M tokens. Never fetched over the network.
//!
//! Defaults follow CLAUDE.md: cache reads bill at 10% of base input and cache
//! writes at 125%. Two refinements from Anthropic's published prices:
//! - Opus 5.5 and Fable 5.1 have cheaper cache reads than 10% (overridden below).
//! - Claude Code mostly writes 1-hour cache entries, which bill at 2x base input.
//!   `usage.cache_creation.ephemeral_1h_input_tokens` tells us how many.
//!
//! An optional local `~/.cc-deskmeter/pricing.json` adds or replaces prices
//! (see `Pricing`), e.g. for a model released after this build.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::types::TokenCounts;

const CACHE_WRITE_5M: f64 = 1.25;
const CACHE_WRITE_1H: f64 = 2.0;
const CACHE_READ: f64 = 0.1;

/// Serialized as the price table the archive's costs were computed with.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelRates {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write_5m: f64,
    pub cache_write_1h: f64,
    /// Applied to every rate when the response ran in fast mode (`usage.speed == "fast"`).
    pub fast_multiplier: f64,
    pub long_context: Option<LongContext>,
}

/// Higher rates for requests whose prompt exceeds `threshold` tokens.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
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
            cache_write_5m: input * CACHE_WRITE_5M,
            cache_write_1h: input * CACHE_WRITE_1H,
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

/// Provider ids such as `anthropic.claude-sonnet-4-5-v1:0` compare like
/// `claude-sonnet-4-5`.
fn normalize(model: &str) -> String {
    model.to_ascii_lowercase().replace(['.', '_'], "-")
}

/// The first (most specific) built-in entry whose pattern occurs in `id`.
fn builtin_entry(id: &str) -> Option<(&'static str, ModelRates)> {
    MODEL_PRICES
        .iter()
        .chain(FAMILY_FALLBACKS)
        .find(|(pattern, _)| id.contains(pattern))
        .copied()
}

/// Looks up built-in rates for a model id such as `claude-opus-4-5-20251101`,
/// `anthropic.claude-sonnet-4-5-v1:0` or `claude-opus-4-5@20251101`.
/// The app prices through `Pricing`, which also weighs the local overrides.
#[cfg(test)]
pub fn rates_for(model: &str) -> Option<ModelRates> {
    builtin_entry(&normalize(model)).map(|(_, rates)| rates)
}

/// Built-in prices plus the user's local overrides.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Pricing {
    /// Normalized key and rates, in the file's (sorted) name order.
    overrides: Vec<(String, ModelRates)>,
}

/// One `pricing.json` entry, USD per 1M tokens. Omitted cache prices follow
/// the built-in rules (read 10%, 5-minute write 125%, 1-hour write 200% of
/// input). An override has no long-context tier.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawRates {
    input: f64,
    output: f64,
    cache_read: Option<f64>,
    cache_write_5m: Option<f64>,
    cache_write_1h: Option<f64>,
    fast_multiplier: Option<f64>,
}

impl Pricing {
    /// `~/.cc-deskmeter/pricing.json`, or `~/.claude-pulse/pricing.json` (the
    /// app's name before CC-Deskmeter) while only that one exists.
    pub fn override_path() -> Option<PathBuf> {
        let home = dirs::home_dir()?;
        let path = home.join(".cc-deskmeter").join("pricing.json");
        let legacy = home.join(".claude-pulse").join("pricing.json");
        Some(if !path.exists() && legacy.exists() {
            legacy
        } else {
            path
        })
    }

    /// Parses `pricing.json`: an object mapping a model name to its rates. A
    /// name matches every model id that contains it, like the built-in table.
    pub fn parse(json: &str) -> Result<Self, String> {
        let raw: BTreeMap<String, RawRates> =
            serde_json::from_str(json).map_err(|err| err.to_string())?;
        let mut overrides = Vec::with_capacity(raw.len());
        for (name, raw) in raw {
            let key = normalize(name.trim());
            if key.is_empty() {
                return Err("model names must not be empty".into());
            }
            let invalid = [
                ("input", Some(raw.input)),
                ("output", Some(raw.output)),
                ("cacheRead", raw.cache_read),
                ("cacheWrite5m", raw.cache_write_5m),
                ("cacheWrite1h", raw.cache_write_1h),
                ("fastMultiplier", raw.fast_multiplier),
            ]
            .into_iter()
            .find(|(_, value)| value.is_some_and(|v| !v.is_finite() || v < 0.0));
            if let Some((field, _)) = invalid {
                return Err(format!("\"{name}\": {field} must be a non-negative number"));
            }
            let defaults = ModelRates::new(raw.input, raw.output);
            let rates = ModelRates {
                cache_read: raw.cache_read.unwrap_or(defaults.cache_read),
                cache_write_5m: raw.cache_write_5m.unwrap_or(defaults.cache_write_5m),
                cache_write_1h: raw.cache_write_1h.unwrap_or(defaults.cache_write_1h),
                fast_multiplier: raw.fast_multiplier.unwrap_or(1.0),
                ..defaults
            };
            overrides.push((key, rates));
        }
        Ok(Self { overrides })
    }

    pub fn custom_count(&self) -> usize {
        self.overrides.len()
    }

    /// Every override by its normalized name, in the file's (sorted) name
    /// order: Codex prices its own models from the same file.
    pub fn overrides(&self) -> impl Iterator<Item = (&str, &ModelRates)> {
        self.overrides
            .iter()
            .map(|(key, rates)| (key.as_str(), rates))
    }

    /// The most specific match among the overrides and the built-in table:
    /// the key whose match reaches furthest into the model id, then the
    /// longer key, and an override wins a tie. So `claude-opus-4` reprices
    /// Opus 4 but not Opus 4.5, whose built-in `opus-4-5` reaches further,
    /// while `claude-opus-4-5` replaces that entry.
    /// What cache reads cost as a share of uncached input, over `(model,
    /// cache-read tokens)` weighted by what those tokens would cost as input;
    /// `None` without priced cache reads.
    pub fn cache_read_share<'a>(
        &self,
        reads: impl IntoIterator<Item = (&'a str, u64)>,
    ) -> Option<f64> {
        let (mut read, mut input) = (0.0, 0.0);
        for (model, tokens) in reads {
            if let Some(rates) = self.rates_for(model) {
                read += tokens as f64 * rates.cache_read;
                input += tokens as f64 * rates.input;
            }
        }
        (input > 0.0).then(|| read / input)
    }

    pub fn rates_for(&self, model: &str) -> Option<ModelRates> {
        let id = normalize(model);
        let rank = |key: &str| id.find(key).map(|start| (start + key.len(), key.len()));
        let builtin = builtin_entry(&id).and_then(|(key, rates)| Some((rank(key)?, rates)));
        let custom = self
            .overrides
            .iter()
            .filter_map(|(key, rates)| Some((rank(key)?, *rates)))
            // Of equally specific keys, the first in the file wins.
            .reduce(|best, next| if next.0 > best.0 { next } else { best });
        match (custom, builtin) {
            (Some(custom), Some(builtin)) if custom.0 < builtin.0 => Some(builtin.1),
            (custom, builtin) => custom.or(builtin).map(|(_, rates)| rates),
        }
    }

    /// Prompt size above which a response of `model` counts as long context:
    /// the tier of its current rates, else of its built-in entry (an override
    /// has none), so archived sums keep their tier when an override comes or goes.
    pub fn long_context_threshold(&self, model: &str) -> Option<u64> {
        self.rates_for(model)
            .and_then(|rates| rates.long_context)
            .or_else(|| builtin_entry(&normalize(model)).and_then(|(_, rates)| rates.long_context))
            .map(|tier| tier.threshold)
    }
}

/// API-equivalent cost of one response. `cache_write_1h` is the part of
/// `tokens.cache_creation` written with the 1-hour TTL.
#[cfg(test)]
pub fn cost_usd(rates: &ModelRates, tokens: &TokenCounts, cache_write_1h: u64, fast: bool) -> f64 {
    let long = rates
        .long_context
        .is_some_and(|tier| tokens.prompt() > tier.threshold);
    tier_cost_usd(rates, tokens, cache_write_1h, fast, long)
}

/// Cost of responses that bill alike: summed tokens of fast-mode (or not)
/// responses whose prompts were (or were not) above the long-context threshold.
pub fn tier_cost_usd(
    rates: &ModelRates,
    tokens: &TokenCounts,
    cache_write_1h: u64,
    fast: bool,
    long_context: bool,
) -> f64 {
    let (input, output, scale) = match rates.long_context {
        Some(lc) if long_context => (lc.input, lc.output, lc.input / rates.input),
        _ => (rates.input, rates.output, 1.0),
    };
    let write_1h = cache_write_1h.min(tokens.cache_creation);
    let write_5m = tokens.cache_creation - write_1h;
    let speed = if fast { rates.fast_multiplier } else { 1.0 };

    let usd = tokens.input as f64 * input
        + tokens.output as f64 * output
        + write_5m as f64 * rates.cache_write_5m * scale
        + write_1h as f64 * rates.cache_write_1h * scale
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
        assert!(
            (cost_usd(&rates_for("claude-opus-5-5").unwrap(), &tokens, 0, true)
                - 2.0 * (4.0 + 20.0 + 10.0 + 0.2))
                .abs()
                < 1e-9
        );
    }

    #[test]
    fn cache_reads_bill_a_tenth_of_input_unless_overridden() {
        let read = TokenCounts {
            cache_read: 1_000_000,
            ..Default::default()
        };
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
        let short = TokenCounts {
            input: 100_000,
            ..Default::default()
        };
        let long = TokenCounts {
            input: 100_001,
            ..Default::default()
        };
        assert!((cost_usd(&rates, &short, 0, false) - 0.01).abs() < 1e-9);
        assert!((cost_usd(&rates, &long, 0, false) - 0.0500005).abs() < 1e-9);
    }

    #[test]
    fn summed_tokens_are_priced_by_their_tier_which_outlives_an_override() {
        let pricing =
            Pricing::parse(r#"{ "claude-haiku-5-5": { "input": 1, "output": 5 } }"#).unwrap();
        assert_eq!(
            pricing.rates_for("claude-haiku-5-5").unwrap().long_context,
            None
        );
        assert_eq!(
            pricing.long_context_threshold("claude-haiku-5-5"),
            Some(100_000)
        );
        let builtin = Pricing::default();
        assert_eq!(
            builtin.long_context_threshold("claude-sonnet-4-5"),
            Some(200_000)
        );
        assert_eq!(builtin.long_context_threshold("claude-opus-5-5"), None);
        // Three 100k prompts: the sum is above the threshold, each one is not.
        let rates = rates_for("claude-haiku-5-5").unwrap();
        let sum = TokenCounts {
            input: 300_000,
            ..Default::default()
        };
        assert!((tier_cost_usd(&rates, &sum, 0, false, false) - 0.03).abs() < 1e-12);
        assert!((tier_cost_usd(&rates, &sum, 0, false, true) - 0.15).abs() < 1e-12);
    }

    #[test]
    fn local_overrides_add_and_replace_prices() {
        let pricing = Pricing::parse(
            r#"{
                "claude-opus-6": { "input": 6, "output": 30 },
                "Claude_Opus_6.1": { "input": 7, "output": 35, "cacheRead": 0.5, "cacheWrite1h": 10, "fastMultiplier": 2 },
                "claude-sonnet-5-5": { "input": 1, "output": 5 }
            }"#,
        )
        .unwrap();
        assert_eq!(pricing.custom_count(), 3);
        // New model, defaults derived from input like the built-in table.
        let opus6 = pricing.rates_for("claude-opus-6-20270101").unwrap();
        assert_eq!(
            (opus6.input, opus6.cache_write_5m, opus6.cache_write_1h),
            (6.0, 7.5, 12.0)
        );
        assert!((opus6.cache_read - 0.6).abs() < 1e-12);
        // The most specific matching name wins, whatever the file order or spelling.
        let opus61 = pricing.rates_for("claude-opus-6-1").unwrap();
        assert_eq!(
            (opus61.input, opus61.cache_read, opus61.cache_write_1h),
            (7.0, 0.5, 10.0)
        );
        let tokens = TokenCounts {
            input: 1_000_000,
            cache_creation: 1_000_000,
            ..Default::default()
        };
        assert!((cost_usd(&opus61, &tokens, 1_000_000, true) - 2.0 * (7.0 + 10.0)).abs() < 1e-9);
        // Replaces a built-in price; everything else is unchanged.
        assert_eq!(pricing.rates_for("claude-sonnet-5-5").unwrap().input, 1.0);
        assert_eq!(
            pricing.rates_for("claude-haiku-5-5"),
            rates_for("claude-haiku-5-5")
        );
        assert_eq!(
            Pricing::default().rates_for("claude-opus-6"),
            rates_for("claude-opus-6")
        );
    }

    #[test]
    fn overrides_leave_more_specific_built_in_models_alone() {
        let pricing = Pricing::parse(
            r#"{
                "claude-opus-4": { "input": 15, "output": 75 },
                "claude-sonnet-4": { "input": 3, "output": 15 },
                "opus": { "input": 7, "output": 35 }
            }"#,
        )
        .unwrap();
        // The model a name spells out, in any snapshot, takes the override...
        assert_eq!(
            pricing.rates_for("claude-opus-4-20250514").unwrap().input,
            15.0
        );
        assert_eq!(
            pricing
                .rates_for("claude-sonnet-4-20250514")
                .unwrap()
                .long_context,
            None
        );
        // ...but point releases with a built-in entry of their own keep it.
        for model in [
            "claude-opus-4-5-20251101",
            "claude-opus-4-6",
            "claude-sonnet-4-5-20250929",
            "claude-opus-5-5",
            "claude-3-opus-20240229",
        ] {
            assert_eq!(pricing.rates_for(model), rates_for(model), "{model}");
        }
        // A family name replaces only the family fallback.
        assert_eq!(pricing.rates_for("claude-opus-9").unwrap().input, 7.0);
    }

    #[test]
    fn invalid_override_files_are_rejected() {
        for (json, needle) in [
            (r#"{ "m": { "input": 1 } }"#, "output"),
            (
                r#"{ "m": { "input": 1, "output": 2, "cache_read": 1 } }"#,
                "cache_read",
            ),
            (r#"{ "m": { "input": -1, "output": 2 } }"#, "input"),
            (r#"{ " ": { "input": 1, "output": 2 } }"#, "empty"),
            ("[1, 2]", "map"),
        ] {
            let err = Pricing::parse(json).unwrap_err();
            assert!(err.contains(needle), "{json}: {err}");
        }
    }

    #[test]
    fn cache_read_share_weighs_models_by_their_reads_at_input_price() {
        let pricing = Pricing::default();
        let share = |reads: &[(&str, u64)]| pricing.cache_read_share(reads.iter().copied());
        // Opus 5.5 reads cache at $0.20 against $4 input.
        assert!((share(&[("claude-opus-5-5", 10)]).unwrap() - 0.05).abs() < 1e-12);
        // $0.40 of reads where input would have cost $6; unpriced reads count for nothing.
        let mixed = share(&[
            ("claude-opus-5-5", 1_000_000),
            ("claude-sonnet-5-5", 1_000_000),
            ("<synthetic>", 5_000_000),
        ]);
        assert!((mixed.unwrap() - 0.4 / 6.0).abs() < 1e-12);
        assert_eq!(share(&[("claude-opus-5-5", 0), ("<synthetic>", 9)]), None);
    }
}
