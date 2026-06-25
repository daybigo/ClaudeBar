//! Lector SQLite minimo y de solo lectura para un unico valor de `ItemTable`.
//!
//! Los `state.vscdb` de VS Code / Antigravity son SQLite. No podemos enlazar
//! SQLite en C (el toolchain GNU de esta maquina no trae compilador C), asi que
//! leemos el formato de fichero a mano: localizamos la clave en las hojas de la
//! tabla y reconstruimos su valor siguiendo las paginas de overflow.
//!
//! Referencia: https://www.sqlite.org/fileformat2.html

use std::path::Path;

/// Lee `value` para `key` en la tabla `ItemTable`. Devuelve los bytes crudos
/// (TEXT o BLOB) o None si no existe / el fichero no es SQLite.
pub fn read_item(path: &Path, key: &str) -> Option<Vec<u8>> {
    let file = std::fs::read(path).ok()?;
    if file.len() < 100 || &file[0..16] != b"SQLite format 3\x00" {
        return None;
    }
    let page_size = match u16::from_be_bytes([file[16], file[17]]) {
        1 => 65536usize,
        n => n as usize,
    };
    if page_size == 0 || file.len() < page_size {
        return None;
    }
    let reserved = file[20] as usize;
    let usable = (page_size - reserved) as u64;
    let num_pages = file.len() / page_size;

    for pi in 0..num_pages {
        let base = pi * page_size;
        let hoff = base + if pi == 0 { 100 } else { 0 };
        if hoff + 8 > file.len() {
            continue;
        }
        // Solo nos interesan las hojas de tabla (0x0D); el resto se ignora.
        if file[hoff] != 0x0D {
            continue;
        }
        let num_cells = u16::from_be_bytes([file[hoff + 3], file[hoff + 4]]) as usize;
        let ptr_arr = hoff + 8;
        for c in 0..num_cells {
            let poff = ptr_arr + c * 2;
            if poff + 2 > file.len() {
                break;
            }
            let cell_off = base + u16::from_be_bytes([file[poff], file[poff + 1]]) as usize;
            if let Some(v) = try_cell(&file, page_size, usable, cell_off, key) {
                return Some(v);
            }
        }
    }
    None
}

/// Varint big-endian de SQLite (1..=9 bytes). Devuelve (valor, bytes_leidos).
fn read_varint(buf: &[u8], pos: usize) -> Option<(u64, usize)> {
    let mut result: u64 = 0;
    let mut i = 0;
    while i < 9 {
        let b = *buf.get(pos + i)?;
        if i == 8 {
            result = (result << 8) | b as u64;
            return Some((result, 9));
        }
        result = (result << 7) | (b & 0x7f) as u64;
        if b & 0x80 == 0 {
            return Some((result, i + 1));
        }
        i += 1;
    }
    Some((result, 9))
}

/// Cuantos bytes del payload se guardan en la propia hoja (el resto va a overflow).
fn local_payload_len(p: u64, usable: u64) -> u64 {
    let x = usable - 35;
    if p <= x {
        return p;
    }
    let m = ((usable - 12) * 32 / 255) - 23;
    let k = m + ((p - m) % (usable - 4));
    if k <= x {
        k
    } else {
        m
    }
}

/// Longitud en bytes de un valor segun su serial type de SQLite.
fn serial_len(st: u64) -> usize {
    match st {
        0 | 8 | 9 | 10 | 11 => 0,
        1 => 1,
        2 => 2,
        3 => 3,
        4 => 4,
        5 => 6,
        6 | 7 => 8,
        n if n >= 12 => ((n - 12) / 2) as usize,
        _ => 0,
    }
}

/// Parsea una celda de hoja; si la columna 0 (key) coincide, devuelve la columna 1.
fn try_cell(file: &[u8], page_size: usize, usable: u64, cell_off: usize, key: &str) -> Option<Vec<u8>> {
    let (p, n1) = read_varint(file, cell_off)?;
    let (_rowid, n2) = read_varint(file, cell_off + n1)?;
    let data_off = cell_off + n1 + n2;
    let local = local_payload_len(p, usable) as usize;
    if data_off + local > file.len() {
        return None;
    }

    // Reconstruye el payload completo (local + cadena de overflow).
    let mut payload = Vec::with_capacity(p as usize);
    payload.extend_from_slice(&file[data_off..data_off + local]);
    if (p as usize) > local {
        let ovf_off = data_off + local;
        if ovf_off + 4 > file.len() {
            return None;
        }
        let mut next =
            u32::from_be_bytes([file[ovf_off], file[ovf_off + 1], file[ovf_off + 2], file[ovf_off + 3]])
                as usize;
        let max_pages = file.len() / page_size + 2;
        let mut guard = 0;
        while next != 0 && payload.len() < p as usize && guard < max_pages {
            let pbase = (next - 1) * page_size;
            if pbase + 4 > file.len() {
                break;
            }
            let chunk_next =
                u32::from_be_bytes([file[pbase], file[pbase + 1], file[pbase + 2], file[pbase + 3]])
                    as usize;
            let want = (usable as usize - 4).min(p as usize - payload.len());
            if pbase + 4 + want > file.len() {
                break;
            }
            payload.extend_from_slice(&file[pbase + 4..pbase + 4 + want]);
            next = chunk_next;
            guard += 1;
        }
    }

    // Cabecera del registro: longitud + serial types.
    let (hlen, hn) = read_varint(&payload, 0)?;
    let hlen = hlen as usize;
    if hlen > payload.len() {
        return None;
    }
    let mut serials = Vec::new();
    let mut pos = hn;
    while pos < hlen {
        let (st, n) = read_varint(&payload, pos)?;
        serials.push(st);
        pos += n;
    }
    if serials.len() < 2 {
        return None;
    }

    // Columna 0 = key.
    let mut dpos = hlen;
    let k0 = serial_len(serials[0]);
    if dpos + k0 > payload.len() {
        return None;
    }
    if &payload[dpos..dpos + k0] != key.as_bytes() {
        return None;
    }
    dpos += k0;

    // Columna 1 = value.
    let v1 = serial_len(serials[1]);
    if dpos + v1 > payload.len() {
        return None;
    }
    Some(payload[dpos..dpos + v1].to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> &'static Path {
        Path::new("tests/fixtures/sample.vscdb")
    }

    #[test]
    fn lee_valor_pequeno_sin_overflow() {
        let v = read_item(fixture(), "smallkey").expect("smallkey");
        assert_eq!(String::from_utf8(v).unwrap(), "hello world");
    }

    #[test]
    fn lee_valor_grande_con_overflow() {
        let v = read_item(fixture(), "antigravityAuthStatus").expect("bigkey");
        let s = String::from_utf8(v).unwrap();
        assert_eq!(s.len(), 5010);
        assert!(s.contains("MID_MARKER"));
        assert!(s.starts_with("AAAA"));
        assert!(s.ends_with("BBBB"));
    }

    #[test]
    fn clave_inexistente_devuelve_none() {
        assert!(read_item(fixture(), "no-existe").is_none());
    }
}
