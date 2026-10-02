//! Utilidades para leer los logs .jsonl de Claude Code y Codex sin castigar la PC.
//!
//! Los logs pesan GBs pero lo util es <1% (los eventos de uso); el resto son
//! outputs de herramientas, diffs e imagenes. Por eso:
//!  - cada archivo recuerda hasta que byte se leyo y solo se leen bytes nuevos;
//!  - ese progreso (y los numeros ya extraidos) se guarda en disco, asi que al
//!    reiniciar la app no se relee nada: el escaneo completo pasa una sola vez;
//!  - mientras se escanea, el hilo baja a prioridad "background" de Windows
//!    (I/O y memoria de baja prioridad) para no competir con lo que usa el user
//!    ni llenar la cache de RAM con logs que no vuelven a leerse.

use serde::de::DeserializeOwned;
use serde::Serialize;
use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

/// Cada cuanto se persiste el progreso si hubo cambios (ademas de al cerrar y
/// tras un escaneo grande).
const SAVE_EVERY: Duration = Duration::from_secs(600);
/// Si en una vuelta se leyeron mas bytes que esto, se guarda ya mismo.
const SAVE_AFTER_BYTES: u64 = 32 * 1024 * 1024;

/// Hash FNV-1a de 64 bits: estable entre versiones y plataformas, a diferencia
/// del hasher de std. Se usa como clave de deduplicacion compacta.
pub fn fnv64(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// mtime en nanos desde epoch (0 si no se puede leer).
pub fn mtime_nanos(meta: &std::fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|m| m.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}

/// Lee las lineas completas que hay a partir de `offset` y llama `on_line` con
/// cada una. Devuelve el nuevo offset (justo despues del ultimo '\n'); una
/// linea a medio escribir se deja para la proxima vuelta.
pub fn read_new_lines(path: &Path, offset: u64, mut on_line: impl FnMut(&str)) -> std::io::Result<u64> {
    let mut reader = BufReader::with_capacity(1 << 20, File::open(path)?);
    reader.seek(SeekFrom::Start(offset))?;
    let mut offset = offset;
    let mut buf = Vec::new();
    loop {
        buf.clear();
        let bytes = reader.read_until(b'\n', &mut buf)?;
        if bytes == 0 || buf.last() != Some(&b'\n') {
            break;
        }
        offset += bytes as u64;
        on_line(&String::from_utf8_lossy(&buf));
    }
    Ok(offset)
}

fn cache_dir() -> Option<PathBuf> {
    dirs::cache_dir().map(|d| d.join("com.daybi.claudebar"))
}

/// Carga un cache persistido; None si no existe, esta corrupto o es de otra
/// version del formato.
pub fn load<T: DeserializeOwned>(name: &str) -> Option<T> {
    if cfg!(test) {
        return None; // los tests no tocan el cache real del user
    }
    let bytes = std::fs::read(cache_dir()?.join(name)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Guarda de forma atomica (archivo temporal + rename) para que un corte de
/// luz no deje un cache a medias.
pub fn save<T: Serialize>(name: &str, value: &T) {
    if cfg!(test) {
        return;
    }
    let Some(dir) = cache_dir() else { return };
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let Ok(bytes) = serde_json::to_vec(value) else { return };
    let tmp = dir.join(format!("{name}.tmp"));
    if std::fs::write(&tmp, bytes).is_ok() {
        let _ = std::fs::rename(&tmp, dir.join(name));
    }
}

/// Decide cuando persistir: no en cada vuelta de 60s (seria escribir MBs por
/// minuto), sino tras escaneos grandes, cada 10 min si hubo cambios y al cerrar.
#[derive(Default)]
pub struct SavePolicy {
    dirty: bool,
    last_save: Option<Instant>,
}

impl SavePolicy {
    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    pub fn should_save(&self, bytes_read: u64, force: bool) -> bool {
        self.dirty
            && (force
                || bytes_read >= SAVE_AFTER_BYTES
                || self.last_save.map_or(true, |t| t.elapsed() >= SAVE_EVERY))
    }

    pub fn saved(&mut self) {
        self.dirty = false;
        self.last_save = Some(Instant::now());
    }
}

/// Mientras vive, el hilo actual corre en modo background de Windows: I/O y
/// memoria de baja prioridad. En otras plataformas no hace nada.
pub struct BackgroundIo {
    #[cfg(windows)]
    active: bool,
}

#[cfg(windows)]
mod win {
    pub const THREAD_MODE_BACKGROUND_BEGIN: i32 = 0x0001_0000;
    pub const THREAD_MODE_BACKGROUND_END: i32 = 0x0002_0000;
    #[link(name = "kernel32")]
    extern "system" {
        pub fn GetCurrentThread() -> isize;
        pub fn SetThreadPriority(thread: isize, priority: i32) -> i32;
    }
}

impl BackgroundIo {
    pub fn begin() -> Self {
        #[cfg(windows)]
        {
            // Falla si el hilo ya estaba en background; en ese caso no lo
            // terminamos al salir para no pisar al que lo activo.
            let active = unsafe {
                win::SetThreadPriority(win::GetCurrentThread(), win::THREAD_MODE_BACKGROUND_BEGIN) != 0
            };
            Self { active }
        }
        #[cfg(not(windows))]
        {
            Self {}
        }
    }
}

impl Drop for BackgroundIo {
    fn drop(&mut self) {
        #[cfg(windows)]
        if self.active {
            unsafe {
                win::SetThreadPriority(win::GetCurrentThread(), win::THREAD_MODE_BACKGROUND_END);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn fnv_is_stable() {
        assert_eq!(fnv64(""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv64("a"), 0xaf63_dc4c_8601_ec8c);
    }

    #[test]
    fn reads_only_complete_new_lines() {
        let path = std::env::temp_dir().join(format!("claudebar-logscan-{}.jsonl", std::process::id()));
        std::fs::write(&path, "uno\ndos\ntre").unwrap();
        let mut seen = Vec::new();
        let off = read_new_lines(&path, 0, |l| seen.push(l.trim_end().to_string())).unwrap();
        assert_eq!(seen, ["uno", "dos"]);
        assert_eq!(off, 8);
        let mut f = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(f, "s").unwrap();
        drop(f);
        seen.clear();
        let off = read_new_lines(&path, off, |l| seen.push(l.trim_end().to_string())).unwrap();
        assert_eq!(seen, ["tres"]);
        assert_eq!(off, 13);
        std::fs::remove_file(path).unwrap();
    }
}
