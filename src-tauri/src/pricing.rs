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

// Precios oficiales de Anthropic, verificados 2026-10-01 (docs de la API).
// Escritura de cache: 5m = 1.25x el input, 1h = 2x (ver cost_usd). La lectura
// de cache ya no es siempre 0.1x: Fable 5.1 la cobra a 0.025x ($0.25) y
// Opus 5.5 a 0.05x ($0.20), por eso va explicita por modelo.
const fn price(input: f64, output: f64, cache_read: f64) -> ModelPrice {
    ModelPrice { input, output, cache_write_5m: input * 1.25, cache_read }
}
const FABLE51: ModelPrice = price(10.0, 50.0, 0.25);
const FABLE5: ModelPrice = price(10.0, 50.0, 1.00);
const OPUS55: ModelPrice = price(4.0, 20.0, 0.20);
const OPUS5: ModelPrice = price(5.0, 25.0, 0.50);
const OPUS_LEGACY: ModelPrice = price(15.0, 75.0, 1.50);
const SONNET5: ModelPrice = price(2.0, 10.0, 0.20);
const SONNET4: ModelPrice = price(3.0, 15.0, 0.30);
const HAIKU: ModelPrice = price(1.0, 5.0, 0.10);
const FREE: ModelPrice = price(0.0, 0.0, 0.0);

/// Devuelve el precio para un id de modelo. Coincidencia por substring para
/// tolerar sufijos de fecha y de contexto (claude-opus-5-5[1m], ...). El orden
/// importa: "fable-5-1" antes que "fable-5", "opus-5-5" antes que "opus-5".
pub fn price_for(model: &str) -> ModelPrice {
    let m = model.to_ascii_lowercase();
    if m.starts_with('<') {
        // "<synthetic>": mensajes que arma Claude Code, no se cobran.
        FREE
    } else if m.contains("fable-5-1") || m.contains("mythos-5-1") {
        FABLE51
    } else if m.contains("fable") || m.contains("mythos") {
        FABLE5
    } else if m.contains("opus-5-5") {
        OPUS55
    } else if m.contains("opus") {
        // Opus 4.5 a 5 valen $5/$25; las viejas (4.1, 4.0, claude-opus-4-2025xxxx, 3) $15/$75.
        if m.contains("opus-4-1") || m.contains("opus-4-0") || m.contains("opus-4-2") || m.contains("opus-3") {
            OPUS_LEGACY
        } else {
            OPUS5
        }
    } else if m.contains("haiku") {
        HAIKU
    } else if m.contains("sonnet-5") {
        SONNET5
    } else {
        // Sonnet 4.x y desconocidos.
        SONNET4
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
