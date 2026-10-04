//! Result values as the grid shows them (SPEC.md, Databases: Results): every
//! value becomes a neutral `Cell`, never a driver's type.
//!
//! PostgreSQL sends results in its binary format, decoded here by type. Text
//! comes out the way PostgreSQL would print it, so a copied value pastes back
//! into SQL unchanged; `timestamptz` is the exception and is shown in the
//! Mac's time zone, with its offset.

use chrono::{Duration, Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone};
use tokio_postgres::types::{Field, Kind, Type};

use super::export::ExportValue;
use crate::models::{Cell, CellValue, ColumnKind};

/// Text longer than this is cut for the trip to the window.
pub const CUT_AT: usize = 64 * 1024;
/// How much of a binary value is shown, as hex.
pub const BYTES_SHOWN: usize = 4 * 1024;
/// Integers beyond 2^53 travel as text, so JavaScript cannot round them.
const SAFE_INTEGER: i64 = 1 << 53;

/// A value that could not be decoded: its type is not one Brainiac knows.
#[derive(Debug)]
pub struct Unsupported;

type Decoded<T> = Result<T, Unsupported>;

/// Text cut at `CUT_AT` bytes, on a character boundary.
pub fn text_cell(text: String) -> Cell {
    if text.len() <= CUT_AT {
        return Cell::Text(text);
    }
    let mut end = CUT_AT;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    Cell::Value(CellValue::Cut {
        text: text[..end].to_string(),
        length: text.len() as u64,
    })
}

pub fn bytes_cell(bytes: &[u8]) -> Cell {
    Cell::Value(CellValue::Bytes {
        size: bytes.len() as u64,
        hex: hex(&bytes[..bytes.len().min(BYTES_SHOWN)]),
    })
}

pub fn integer_cell(v: i64) -> Cell {
    if (-SAFE_INTEGER..=SAFE_INTEGER).contains(&v) {
        Cell::Number(v as f64)
    } else {
        Cell::Text(v.to_string())
    }
}

pub fn float_cell(v: f64) -> Cell {
    if v.is_finite() {
        Cell::Number(v)
    } else {
        Cell::Text(float_text(v))
    }
}

fn float_text(v: f64) -> String {
    if v.is_nan() {
        "NaN".into()
    } else if v.is_infinite() {
        if v > 0.0 { "Infinity" } else { "-Infinity" }.into()
    } else {
        v.to_string()
    }
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// Types `text_of` prints that are neither numbers nor dates.
const TEXT_TYPES: &[Type] = &[
    Type::TEXT,
    Type::VARCHAR,
    Type::BPCHAR,
    Type::NAME,
    Type::UNKNOWN,
    Type::XML,
    Type::REFCURSOR,
    Type::CHAR,
    Type::XID,
    Type::CID,
    Type::REGCLASS,
    Type::REGTYPE,
    Type::REGPROC,
    Type::INET,
    Type::CIDR,
    Type::MACADDR,
    Type::MACADDR8,
    Type::BIT,
    Type::VARBIT,
    Type::PG_LSN,
    Type::XID8,
    Type::TID,
    Type::POINT,
    Type::BOX,
    Type::LSEG,
    Type::CIRCLE,
];

/// What a PostgreSQL column holds, for alignment and the inspector.
pub fn pg_kind(ty: &Type) -> ColumnKind {
    match *ty {
        Type::BOOL => ColumnKind::Bool,
        Type::INT2 | Type::INT4 | Type::INT8 | Type::OID | Type::FLOAT4 | Type::FLOAT8 => {
            ColumnKind::Number
        }
        Type::NUMERIC | Type::MONEY => ColumnKind::Numeric,
        Type::JSON | Type::JSONB | Type::JSONPATH => ColumnKind::Json,
        Type::DATE
        | Type::TIME
        | Type::TIMETZ
        | Type::TIMESTAMP
        | Type::TIMESTAMPTZ
        | Type::INTERVAL => ColumnKind::Temporal,
        Type::UUID => ColumnKind::Uuid,
        Type::BYTEA => ColumnKind::Bytes,
        _ => match ty.kind() {
            Kind::Array(_) => ColumnKind::Array,
            Kind::Domain(base) => pg_kind(base),
            Kind::Enum(_) | Kind::Range(_) | Kind::Multirange(_) | Kind::Composite(_) => {
                ColumnKind::Text
            }
            _ if TEXT_TYPES.contains(ty) || ty.name() == "citext" => ColumnKind::Text,
            _ => ColumnKind::Other,
        },
    }
}

/// The cell for a PostgreSQL value in binary format; `None` is `NULL`.
pub fn pg_cell(ty: &Type, raw: Option<&[u8]>) -> Cell {
    let Some(raw) = raw else {
        return Cell::Null;
    };
    let other = || {
        Cell::Value(CellValue::Other {
            type_name: ty.name().to_string(),
            size: raw.len() as u64,
        })
    };
    let decoded = match *ty {
        Type::BOOL => Reader(raw).u8().map(|b| Cell::Bool(b != 0)),
        Type::INT2 => Reader(raw).i16().map(|v| Cell::Number(v.into())),
        Type::INT4 => Reader(raw).i32().map(|v| Cell::Number(v.into())),
        Type::OID => Reader(raw).u32().map(|v| Cell::Number(v.into())),
        Type::INT8 => Reader(raw).i64().map(integer_cell),
        Type::FLOAT4 => Reader(raw).f32().map(|v| float_cell(v.into())),
        Type::FLOAT8 => Reader(raw).f64().map(float_cell),
        Type::BYTEA => Ok(bytes_cell(raw)),
        Type::JSONB | Type::JSONPATH => jsonb_text(raw).map(|t| text_cell(t.to_string())),
        _ => match ty.kind() {
            Kind::Domain(base) => return pg_cell(base, Some(raw)),
            _ => text_of(ty, raw).map(text_cell),
        },
    };
    decoded.unwrap_or_else(|_| other())
}

/// A whole value for Export, never cut.
pub fn pg_export_value(ty: &Type, raw: Option<&[u8]>) -> ExportValue {
    let Some(raw) = raw else {
        return ExportValue::Null;
    };
    let base = match ty.kind() {
        Kind::Domain(base) => base,
        _ => ty,
    };
    let text = || {
        text_of(base, raw)
            .unwrap_or_else(|_| format!("<{}: cast it to text to export it>", ty.name()))
    };
    match *base {
        Type::BOOL => ExportValue::Bool(raw.first().is_some_and(|b| *b != 0)),
        Type::INT2
        | Type::INT4
        | Type::INT8
        | Type::OID
        | Type::FLOAT4
        | Type::FLOAT8
        | Type::NUMERIC => ExportValue::Number(text()),
        Type::JSON | Type::JSONB => ExportValue::Json(text()),
        _ => ExportValue::Text(text()),
    }
}

/// A value as PostgreSQL would print it, for text columns and for the
/// elements of arrays, ranges, and composite values.
pub fn text_of(ty: &Type, raw: &[u8]) -> Decoded<String> {
    let mut r = Reader(raw);
    let text = match *ty {
        Type::BOOL => if r.u8()? != 0 { "t" } else { "f" }.to_string(),
        Type::INT2 => r.i16()?.to_string(),
        Type::INT4 => r.i32()?.to_string(),
        Type::INT8 => r.i64()?.to_string(),
        Type::OID | Type::XID | Type::CID | Type::REGCLASS | Type::REGTYPE | Type::REGPROC => {
            r.u32()?.to_string()
        }
        Type::FLOAT4 => float_text(r.f32()?.into()),
        Type::FLOAT8 => float_text(r.f64()?),
        Type::NUMERIC => numeric_text(raw)?,
        Type::MONEY => money_text(r.i64()?),
        Type::TEXT
        | Type::VARCHAR
        | Type::BPCHAR
        | Type::NAME
        | Type::UNKNOWN
        | Type::XML
        | Type::JSON
        | Type::REFCURSOR => utf8(raw)?.to_string(),
        Type::JSONB | Type::JSONPATH => jsonb_text(raw)?.to_string(),
        Type::CHAR => char::from(r.u8()?).to_string(),
        Type::BYTEA => format!("\\x{}", hex(raw)),
        Type::UUID => uuid_text(raw)?,
        Type::DATE => date_text(r.i32()?),
        Type::TIME => time_text(r.i64()?),
        Type::TIMETZ => {
            let micros = r.i64()?;
            let west = r.i32()?;
            format!("{}{}", time_text(micros), offset_text(-west))
        }
        Type::TIMESTAMP => timestamp_text(r.i64()?, false),
        Type::TIMESTAMPTZ => timestamp_text(r.i64()?, true),
        Type::INTERVAL => {
            let micros = r.i64()?;
            let days = r.i32()?;
            let months = r.i32()?;
            interval_text(months, days, micros)
        }
        Type::INET | Type::CIDR => inet_text(raw, *ty == Type::CIDR)?,
        Type::MACADDR | Type::MACADDR8 => raw
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<Vec<_>>()
            .join(":"),
        Type::BIT | Type::VARBIT => bits_text(raw)?,
        Type::XID8 => r.u64()?.to_string(),
        Type::TID => {
            let block = r.u32()?;
            format!("({block},{})", r.u16()?)
        }
        Type::PG_LSN => {
            let lsn = r.u64()?;
            format!("{:X}/{:X}", lsn >> 32, lsn & 0xFFFF_FFFF)
        }
        Type::POINT => point_text(&mut r)?,
        Type::BOX => {
            let upper = point_text(&mut r)?;
            format!("{upper},{}", point_text(&mut r)?)
        }
        Type::LSEG => {
            let a = point_text(&mut r)?;
            format!("[{a},{}]", point_text(&mut r)?)
        }
        Type::CIRCLE => {
            let center = point_text(&mut r)?;
            format!("<{center},{}>", float_text(r.f64()?))
        }
        _ => match ty.kind() {
            Kind::Enum(_) => utf8(raw)?.to_string(),
            Kind::Domain(base) => text_of(base, raw)?,
            Kind::Array(element) => array_text(element, raw)?,
            Kind::Range(element) => range_text(element, raw)?,
            Kind::Multirange(element) => multirange_text(element, raw)?,
            Kind::Composite(fields) => composite_text(fields, raw)?,
            // `citext` and similar extension types send their text.
            _ if ty.name() == "citext" => utf8(raw)?.to_string(),
            _ => return Err(Unsupported),
        },
    };
    Ok(text)
}

/// Big-endian reads from a value's bytes.
struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Decoded<&'a [u8]> {
        if self.0.len() < n {
            return Err(Unsupported);
        }
        let (head, rest) = self.0.split_at(n);
        self.0 = rest;
        Ok(head)
    }
    fn array<const N: usize>(&mut self) -> Decoded<[u8; N]> {
        self.take(N)?.try_into().map_err(|_| Unsupported)
    }
    fn u8(&mut self) -> Decoded<u8> {
        Ok(self.take(1)?[0])
    }
    fn i16(&mut self) -> Decoded<i16> {
        Ok(i16::from_be_bytes(self.array()?))
    }
    fn u16(&mut self) -> Decoded<u16> {
        Ok(u16::from_be_bytes(self.array()?))
    }
    fn i32(&mut self) -> Decoded<i32> {
        Ok(i32::from_be_bytes(self.array()?))
    }
    fn u32(&mut self) -> Decoded<u32> {
        Ok(u32::from_be_bytes(self.array()?))
    }
    fn i64(&mut self) -> Decoded<i64> {
        Ok(i64::from_be_bytes(self.array()?))
    }
    fn u64(&mut self) -> Decoded<u64> {
        Ok(u64::from_be_bytes(self.array()?))
    }
    fn f32(&mut self) -> Decoded<f32> {
        Ok(f32::from_be_bytes(self.array()?))
    }
    fn f64(&mut self) -> Decoded<f64> {
        Ok(f64::from_be_bytes(self.array()?))
    }
    /// A length-prefixed value, `None` for `NULL` (length -1).
    fn value(&mut self) -> Decoded<Option<&'a [u8]>> {
        let len = self.i32()?;
        if len < 0 {
            return Ok(None);
        }
        self.take(len as usize).map(Some)
    }
}

fn utf8(raw: &[u8]) -> Decoded<&str> {
    std::str::from_utf8(raw).map_err(|_| Unsupported)
}

/// `jsonb` and `jsonpath` send a version byte, 1, before their text.
fn jsonb_text(raw: &[u8]) -> Decoded<&str> {
    match raw.split_first() {
        Some((1, text)) => utf8(text),
        _ => Err(Unsupported),
    }
}

fn uuid_text(raw: &[u8]) -> Decoded<String> {
    let bytes: [u8; 16] = raw.try_into().map_err(|_| Unsupported)?;
    Ok(uuid::Uuid::from_bytes(bytes).hyphenated().to_string())
}

/// `numeric`: base-10000 digits with a weight, a sign, and a display scale.
pub fn numeric_text(raw: &[u8]) -> Decoded<String> {
    let mut r = Reader(raw);
    let ndigits = r.i16()?;
    let weight = r.i16()? as i32;
    let sign = r.u16()?;
    let dscale = r.u16()? as usize;
    match sign {
        0xC000 => return Ok("NaN".into()),
        0xD000 => return Ok("Infinity".into()),
        0xF000 => return Ok("-Infinity".into()),
        0x0000 | 0x4000 => {}
        _ => return Err(Unsupported),
    }
    let digits = (0..ndigits.max(0))
        .map(|_| r.i16())
        .collect::<Decoded<Vec<i16>>>()?;
    let digit = |k: i32| -> i16 {
        if k < 0 {
            0
        } else {
            digits.get(k as usize).copied().unwrap_or(0)
        }
    };
    let mut s = String::new();
    if sign == 0x4000 {
        s.push('-');
    }
    if weight < 0 {
        s.push('0');
    } else {
        for k in 0..=weight {
            if k == 0 {
                s.push_str(&digit(k).to_string());
            } else {
                s.push_str(&format!("{:04}", digit(k)));
            }
        }
    }
    if dscale > 0 {
        let mut fraction = String::with_capacity(dscale + 4);
        let mut k = weight + 1;
        while fraction.len() < dscale {
            fraction.push_str(&format!("{:04}", digit(k)));
            k += 1;
        }
        fraction.truncate(dscale);
        s.push('.');
        s.push_str(&fraction);
    }
    Ok(s)
}

fn money_text(cents: i64) -> String {
    let sign = if cents < 0 { "-" } else { "" };
    let abs = cents.unsigned_abs();
    format!("{sign}{}.{:02}", abs / 100, abs % 100)
}

fn epoch() -> NaiveDateTime {
    NaiveDate::from_ymd_opt(2000, 1, 1)
        .expect("a valid date")
        .and_hms_opt(0, 0, 0)
        .expect("a valid time")
}

/// A date as PostgreSQL prints it: `2026-10-04`, years before 1 as `BC`.
fn ymd(date: NaiveDate) -> String {
    use chrono::Datelike;
    let year = date.year();
    if year <= 0 {
        format!("{:04}-{:02}-{:02} BC", 1 - year, date.month(), date.day())
    } else {
        format!("{:04}-{:02}-{:02}", year, date.month(), date.day())
    }
}

fn date_text(days: i32) -> String {
    match days {
        i32::MAX => "infinity".into(),
        i32::MIN => "-infinity".into(),
        _ => match epoch()
            .date()
            .checked_add_signed(Duration::days(days.into()))
        {
            Some(date) => ymd(date),
            None => days.to_string(),
        },
    }
}

/// `HH:MM:SS` with the fraction PostgreSQL shows: none, or up to six digits
/// without trailing zeros.
fn clock(time: NaiveTime) -> String {
    use chrono::Timelike;
    let base = format!(
        "{:02}:{:02}:{:02}",
        time.hour(),
        time.minute(),
        time.second()
    );
    let micros = time.nanosecond() / 1000;
    if micros == 0 {
        base
    } else {
        let fraction = format!("{micros:06}");
        format!("{base}.{}", fraction.trim_end_matches('0'))
    }
}

fn time_text(micros: i64) -> String {
    // 24:00:00 is a valid `time`, past what `NaiveTime` holds.
    if micros == 86_400_000_000 {
        return "24:00:00".into();
    }
    let secs = micros.div_euclid(1_000_000);
    let nanos = (micros.rem_euclid(1_000_000) * 1000) as u32;
    NaiveTime::from_num_seconds_from_midnight_opt(secs as u32, nanos)
        .map(clock)
        .unwrap_or_else(|| micros.to_string())
}

/// An offset east of UTC as PostgreSQL prints it: `+02`, `-05:30`.
fn offset_text(east_seconds: i32) -> String {
    let sign = if east_seconds < 0 { '-' } else { '+' };
    let abs = east_seconds.unsigned_abs();
    let (h, m, s) = (abs / 3600, abs % 3600 / 60, abs % 60);
    match (m, s) {
        (0, 0) => format!("{sign}{h:02}"),
        (_, 0) => format!("{sign}{h:02}:{m:02}"),
        _ => format!("{sign}{h:02}:{m:02}:{s:02}"),
    }
}

fn timestamp_text(micros: i64, with_zone: bool) -> String {
    match micros {
        i64::MAX => return "infinity".into(),
        i64::MIN => return "-infinity".into(),
        _ => {}
    }
    let Some(at) = epoch().checked_add_signed(Duration::microseconds(micros)) else {
        return micros.to_string();
    };
    if with_zone {
        let local = Local.from_utc_datetime(&at);
        let naive = local.naive_local();
        let date = ymd(naive.date());
        let (date, era) = match date.strip_suffix(" BC") {
            Some(d) => (d.to_string(), " BC"),
            None => (date, ""),
        };
        format!(
            "{date} {}{}{era}",
            clock(naive.time()),
            offset_text(local.offset().local_minus_utc())
        )
    } else {
        let date = ymd(at.date());
        match date.strip_suffix(" BC") {
            Some(d) => format!("{d} {} BC", clock(at.time())),
            None => format!("{date} {}", clock(at.time())),
        }
    }
}

/// An interval in PostgreSQL's default style: `1 year 2 mons 3 days 04:05:06`.
pub fn interval_text(months: i32, days: i32, micros: i64) -> String {
    let mut parts = Vec::new();
    let unit = |n: i32, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
    let (years, mons) = (months / 12, months % 12);
    if years != 0 {
        parts.push(unit(years, "year", "years"));
    }
    if mons != 0 {
        parts.push(unit(mons, "mon", "mons"));
    }
    if days != 0 {
        parts.push(unit(days, "day", "days"));
    }
    if micros != 0 || parts.is_empty() {
        let sign = if micros < 0 {
            "-"
        } else if months < 0 || days < 0 {
            "+"
        } else {
            ""
        };
        let abs = micros.unsigned_abs();
        let (secs, frac) = (abs / 1_000_000, abs % 1_000_000);
        let mut time = format!(
            "{sign}{:02}:{:02}:{:02}",
            secs / 3600,
            secs % 3600 / 60,
            secs % 60
        );
        if frac != 0 {
            time.push('.');
            time.push_str(format!("{frac:06}").trim_end_matches('0'));
        }
        parts.push(time);
    }
    parts.join(" ")
}

fn inet_text(raw: &[u8], cidr: bool) -> Decoded<String> {
    let mut r = Reader(raw);
    let family = r.u8()?;
    let bits = r.u8()?;
    let _is_cidr = r.u8()?;
    let len = r.u8()? as usize;
    let addr = r.take(len)?;
    let (text, full) = match (family, len) {
        (2, 4) => (
            std::net::Ipv4Addr::new(addr[0], addr[1], addr[2], addr[3]).to_string(),
            32,
        ),
        (3, 16) => {
            let octets: [u8; 16] = addr.try_into().map_err(|_| Unsupported)?;
            (std::net::Ipv6Addr::from(octets).to_string(), 128)
        }
        _ => return Err(Unsupported),
    };
    Ok(if cidr || bits != full {
        format!("{text}/{bits}")
    } else {
        text
    })
}

fn bits_text(raw: &[u8]) -> Decoded<String> {
    let mut r = Reader(raw);
    let len = r.i32()?.max(0) as usize;
    let bytes = r.0;
    if bytes.len() * 8 < len {
        return Err(Unsupported);
    }
    Ok((0..len)
        .map(|i| {
            if bytes[i / 8] & (0x80 >> (i % 8)) != 0 {
                '1'
            } else {
                '0'
            }
        })
        .collect())
}

fn point_text(r: &mut Reader<'_>) -> Decoded<String> {
    let x = r.f64()?;
    let y = r.f64()?;
    Ok(format!("({},{})", float_text(x), float_text(y)))
}

/// An element of an array or composite value, quoted when PostgreSQL would.
fn quoted_element(text: &str, delimiter: char) -> String {
    let needs_quotes = text.is_empty()
        || text.eq_ignore_ascii_case("NULL")
        || text
            .chars()
            .any(|c| c == delimiter || "{}()\"\\".contains(c) || c.is_whitespace());
    if !needs_quotes {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        if c == '"' || c == '\\' {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    out
}

/// An array: `{1,2,NULL}`, nested braces for each dimension.
fn array_text(element: &Type, raw: &[u8]) -> Decoded<String> {
    let mut r = Reader(raw);
    let ndim = r.i32()?;
    let _has_nulls = r.i32()?;
    let _element_oid = r.u32()?;
    if ndim == 0 {
        return Ok("{}".into());
    }
    let mut dims = Vec::new();
    for _ in 0..ndim {
        let size = r.i32()?.max(0) as usize;
        let _lower = r.i32()?;
        dims.push(size);
    }
    let total: usize = dims.iter().product();
    let mut values = Vec::with_capacity(total.min(100_000));
    for _ in 0..total {
        values.push(match r.value()? {
            None => "NULL".to_string(),
            Some(v) => quoted_element(&text_of(element, v)?, ','),
        });
    }
    fn nest(dims: &[usize], values: &mut std::slice::Iter<'_, String>) -> String {
        let Some((&size, rest)) = dims.split_first() else {
            return values.next().cloned().unwrap_or_default();
        };
        let items: Vec<String> = (0..size).map(|_| nest(rest, values)).collect();
        format!("{{{}}}", items.join(","))
    }
    Ok(nest(&dims, &mut values.iter()))
}

/// A range: `[1,10)`, `empty`.
fn range_text(element: &Type, raw: &[u8]) -> Decoded<String> {
    let mut r = Reader(raw);
    let flags = r.u8()?;
    if flags & 0x01 != 0 {
        return Ok("empty".into());
    }
    let mut bound = |infinite: bool| -> Decoded<String> {
        if infinite {
            return Ok(String::new());
        }
        let len = r.i32()?.max(0) as usize;
        let value = r.take(len)?;
        Ok(quoted_element(&text_of(element, value)?, ','))
    };
    let lower = bound(flags & 0x08 != 0)?;
    let upper = bound(flags & 0x10 != 0)?;
    Ok(format!(
        "{}{lower},{upper}{}",
        if flags & 0x02 != 0 { '[' } else { '(' },
        if flags & 0x04 != 0 { ']' } else { ')' }
    ))
}

/// A multirange: `{[1,3),[5,7)}`, each range with its length before it.
fn multirange_text(element: &Type, raw: &[u8]) -> Decoded<String> {
    let mut r = Reader(raw);
    let count = r.i32()?.max(0) as usize;
    let mut parts = Vec::with_capacity(count.min(10_000));
    for _ in 0..count {
        let value = r.value()?.ok_or(Unsupported)?;
        parts.push(range_text(element, value)?);
    }
    Ok(format!("{{{}}}", parts.join(",")))
}

/// A row value: `(1,"two words",)`.
fn composite_text(fields: &[Field], raw: &[u8]) -> Decoded<String> {
    let mut r = Reader(raw);
    let count = r.i32()?.max(0) as usize;
    let mut parts = Vec::with_capacity(count);
    for i in 0..count {
        let _oid = r.u32()?;
        let ty = fields.get(i).map(Field::type_).ok_or(Unsupported)?;
        parts.push(match r.value()? {
            None => String::new(),
            Some(v) => quoted_element(&text_of(ty, v)?, ','),
        });
    }
    Ok(format!("({})", parts.join(",")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn numeric(ndigits: i16, weight: i16, sign: u16, dscale: u16, digits: &[i16]) -> Vec<u8> {
        let mut out = Vec::new();
        for v in [ndigits as u16, weight as u16, sign, dscale] {
            out.extend_from_slice(&v.to_be_bytes());
        }
        for d in digits {
            out.extend_from_slice(&d.to_be_bytes());
        }
        out
    }

    #[test]
    fn numerics_print_exactly() {
        assert_eq!(numeric_text(&numeric(0, 0, 0, 0, &[])).unwrap(), "0");
        assert_eq!(numeric_text(&numeric(0, 0, 0, 2, &[])).unwrap(), "0.00");
        assert_eq!(
            numeric_text(&numeric(2, 0, 0x4000, 2, &[12, 3400])).unwrap(),
            "-12.34"
        );
        // 123456789.000000001
        assert_eq!(
            numeric_text(&numeric(6, 2, 0, 9, &[1, 2345, 6789, 0, 0, 1000])).unwrap(),
            "123456789.000000001"
        );
        // 0.00001 has weight -2.
        assert_eq!(
            numeric_text(&numeric(1, -2, 0, 5, &[1000])).unwrap(),
            "0.00001"
        );
        assert_eq!(numeric_text(&numeric(0, 0, 0xC000, 0, &[])).unwrap(), "NaN");
        assert_eq!(
            numeric_text(&numeric(0, 0, 0xF000, 0, &[])).unwrap(),
            "-Infinity"
        );
        // 10000 is one digit of weight 1.
        assert_eq!(numeric_text(&numeric(1, 1, 0, 0, &[1])).unwrap(), "10000");
    }

    #[test]
    fn intervals_print_like_postgres() {
        assert_eq!(
            interval_text(14, 3, 14_706_000_000),
            "1 year 2 mons 3 days 04:05:06"
        );
        assert_eq!(interval_text(0, 0, 0), "00:00:00");
        assert_eq!(interval_text(0, -1, 7_380_000_000), "-1 days +02:03:00");
        assert_eq!(interval_text(1, 1, 1_500_000), "1 mon 1 day 00:00:01.5");
        assert_eq!(interval_text(0, 0, -60_000_000), "-00:01:00");
    }

    #[test]
    fn dates_times_and_offsets() {
        assert_eq!(date_text(0), "2000-01-01");
        assert_eq!(date_text(i32::MAX), "infinity");
        let year_zero = NaiveDate::from_ymd_opt(0, 12, 31).unwrap() - epoch().date();
        assert_eq!(date_text(year_zero.num_days() as i32), "0001-12-31 BC");
        assert_eq!(time_text(45_296_000_100), "12:34:56.0001");
        assert_eq!(time_text(86_400_000_000), "24:00:00");
        assert_eq!(timestamp_text(1_500_000, false), "2000-01-01 00:00:01.5");
        assert_eq!(offset_text(7200), "+02");
        assert_eq!(offset_text(-19_800), "-05:30");
    }

    #[test]
    fn elements_are_quoted_when_postgres_would() {
        assert_eq!(quoted_element("abc", ','), "abc");
        assert_eq!(quoted_element("a b", ','), "\"a b\"");
        assert_eq!(quoted_element("", ','), "\"\"");
        assert_eq!(quoted_element("null", ','), "\"null\"");
        assert_eq!(quoted_element("a\"b\\", ','), "\"a\\\"b\\\\\"");
    }

    #[test]
    fn cells_keep_big_integers_and_cut_long_text() {
        assert_eq!(integer_cell(42), Cell::Number(42.0));
        assert_eq!(
            integer_cell(9_007_199_254_740_993),
            Cell::Text("9007199254740993".into())
        );
        assert_eq!(float_cell(f64::NAN), Cell::Text("NaN".into()));
        let long = "é".repeat(CUT_AT);
        match text_cell(long.clone()) {
            Cell::Value(CellValue::Cut { text, length }) => {
                assert!(text.len() <= CUT_AT);
                assert_eq!(length, long.len() as u64);
            }
            other => panic!("not cut: {other:?}"),
        }
        match bytes_cell(&[0xde, 0xad]) {
            Cell::Value(CellValue::Bytes { size, hex }) => {
                assert_eq!((size, hex.as_str()), (2, "dead"));
            }
            other => panic!("{other:?}"),
        }
    }
}
