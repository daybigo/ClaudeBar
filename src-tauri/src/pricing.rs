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

const OPUS: ModelPrice = ModelPrice { input: 15.0, output: 75.0, cache_write_5m: 18.75, cache_read: 1.50 };
const SONNET: ModelPrice = ModelPrice { input: 3.0, output: 15.0, cache_write_5m: 3.75, cache_read: 0.30 };
const HAIKU: ModelPrice = ModelPrice { input: 1.0, output: 5.0, cache_write_5m: 1.25, cache_read: 0.10 };

/// Devuelve el precio para un id de modelo. Coincidencia por substring para
/// tolerar sufijos de version (claude-opus-4-8, claude-sonnet-4-6, etc.).
pub fn price_for(model: &str) -> ModelPrice {
    let m = model.to_ascii_lowercase();
    if m.contains("opus") {
        OPUS
    } else if m.contains("haiku") {
        HAIKU
    } else if m.contains("sonnet") {
        SONNET
    } else if m.contains("fable") {
        // Fable es un modelo rapido; lo estimamos a nivel Sonnet.
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
