//! Tabla de precios por modelo (USD por millon de tokens).
//! Los costos de los logs locales son ESTIMACIONES: Claude Code no guarda el
//! costo real, asi que lo calculamos a partir de los tokens. Ajusta estos
//! valores si Anthropic cambia los precios.

/// Precios en USD por 1 millon de tokens.
#[derive(Clone, Copy)]
pub struct ModelPrice {
    pub input: f64,
    pub output: f64,
    /// Escritura de cache efimera de 5 minutos.
    pub cache_write_5m: f64,
    /// Lectura de cache (lo mas barato).
    pub cache_read: f64,
}

// Precios oficiales de Anthropic (fuente: la tabla de LiteLLM que usa ccusage,
// verificado 2026-07). La familia Claude 5 se reprecio: Opus bajo de $15/$75
// (era 4.1) a $5/$25 (4.8), y Fable 5 es el tier premium por encima de Opus.
// En toda la familia las tarifas de cache son multiplos del input: escritura
// 5m = 1.25x, escritura 1h = 2x (ver cost_usd), lectura = 0.1x.
const FABLE5: ModelPrice = ModelPrice { input: 10.0, output: 50.0, cache_write_5m: 12.50, cache_read: 1.00 };
const OPUS48: ModelPrice = ModelPrice { input: 5.0, output: 25.0, cache_write_5m: 6.25, cache_read: 0.50 };
const OPUS_LEGACY: ModelPrice = ModelPrice { input: 15.0, output: 75.0, cache_write_5m: 18.75, cache_read: 1.50 };
const SONNET: ModelPrice = ModelPrice { input: 3.0, output: 15.0, cache_write_5m: 3.75, cache_read: 0.30 };
const HAIKU: ModelPrice = ModelPrice { input: 1.0, output: 5.0, cache_write_5m: 1.25, cache_read: 0.10 };

/// Devuelve el precio para un id de modelo. Coincidencia por substring para
/// tolerar sufijos de version y fecha (claude-opus-4-8, claude-fable-5, ...).
pub fn price_for(model: &str) -> ModelPrice {
    let m = model.to_ascii_lowercase();
    if m.contains("fable") {
        FABLE5
    } else if m.contains("opus") {
        // Opus 4.8+ vale $5/$25; las versiones viejas (4.1, 4.0, 3) eran $15/$75.
        if m.contains("opus-4-1") || m.contains("opus-4-0") || m.contains("opus-3") {
            OPUS_LEGACY
        } else {
            OPUS48
        }
    } else if m.contains("haiku") {
        HAIKU
    } else if m.contains("sonnet") {
        SONNET
    } else {
        // Desconocido: usamos Sonnet como estimacion media.
        SONNET
    }
}

/// Costo (USD) de un registro a partir de sus tokens.
/// `cache_create_1h` se cobra al doble del precio de input (regla de ccusage).
pub fn cost_usd(
    model: &str,
    input: u64,
    output: u64,
    cache_create_5m: u64,
    cache_create_1h: u64,
    cache_read: u64,
) -> f64 {
    let p = price_for(model);
    let per = |tokens: u64, price_per_m: f64| (tokens as f64) * price_per_m / 1_000_000.0;
    per(input, p.input)
        + per(output, p.output)
        + per(cache_create_5m, p.cache_write_5m)
        + per(cache_create_1h, p.input * 2.0)
        + per(cache_read, p.cache_read)
}
