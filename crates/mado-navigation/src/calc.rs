//! The address bar's calculator (spec 05 §1): arithmetic, unit conversions,
//! time zones and date differences, evaluated locally.

use jiff::civil::{Date, Time};
use jiff::tz::{self, TimeZone};

/// The answer for `text`, or `None` when it is not a calculation.
pub fn evaluate(text: &str, now: &jiff::Zoned) -> Option<String> {
    let text = text.trim();
    arithmetic(text)
        .map(format_number)
        .or_else(|| units(text))
        .or_else(|| time_zone(text, now))
        .or_else(|| dates(text, now))
}

/// Up to 10 significant digits, without trailing zeros.
fn format_number(v: f64) -> String {
    if v == 0.0 {
        return "0".into();
    }
    if (v.round() - v).abs() < 1e-9 && v.abs() < 1e15 {
        return format!("{}", v.round() as i64);
    }
    let digits = v.abs().log10().floor() as i32 + 1;
    let decimals = usize::try_from((10 - digits).clamp(0, 15)).unwrap_or(0);
    let s = format!("{v:.decimals$}");
    let s = if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_owned()
    } else {
        s
    };
    if s == "-0" { "0".into() } else { s }
}

// Arithmetic: + - * / ^, parentheses, unary minus, × and ÷.

#[derive(Debug, Clone, Copy, PartialEq)]
enum Tok {
    Num(f64),
    Op(char),
    Open,
    Close,
}

fn tokens(text: &str) -> Option<Vec<Tok>> {
    let mut out = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            ' ' => i += 1,
            '0'..='9' | '.' => {
                let start = i;
                while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                    i += 1;
                }
                let s: String = chars[start..i].iter().collect();
                out.push(Tok::Num(s.parse().ok()?));
            }
            '+' | '-' | '*' | '/' | '^' => {
                out.push(Tok::Op(c));
                i += 1;
            }
            '×' => {
                out.push(Tok::Op('*'));
                i += 1;
            }
            '÷' => {
                out.push(Tok::Op('/'));
                i += 1;
            }
            '(' => {
                out.push(Tok::Open);
                i += 1;
            }
            ')' => {
                out.push(Tok::Close);
                i += 1;
            }
            _ => return None,
        }
    }
    Some(out)
}

struct Parser {
    toks: Vec<Tok>,
    at: usize,
    binary: usize,
}

impl Parser {
    fn peek(&self) -> Option<Tok> {
        self.toks.get(self.at).copied()
    }

    fn sum(&mut self) -> Option<f64> {
        let mut v = self.product()?;
        while let Some(Tok::Op(op @ ('+' | '-'))) = self.peek() {
            self.at += 1;
            self.binary += 1;
            let r = self.product()?;
            v = if op == '+' { v + r } else { v - r };
        }
        Some(v)
    }

    fn product(&mut self) -> Option<f64> {
        let mut v = self.unary()?;
        while let Some(Tok::Op(op @ ('*' | '/'))) = self.peek() {
            self.at += 1;
            self.binary += 1;
            let r = self.unary()?;
            v = if op == '*' { v * r } else { v / r };
        }
        Some(v)
    }

    fn unary(&mut self) -> Option<f64> {
        if self.peek() == Some(Tok::Op('-')) {
            self.at += 1;
            return Some(-self.unary()?);
        }
        self.power()
    }

    fn power(&mut self) -> Option<f64> {
        let base = self.atom()?;
        if self.peek() == Some(Tok::Op('^')) {
            self.at += 1;
            self.binary += 1;
            let exp = self.unary()?;
            return Some(base.powf(exp));
        }
        Some(base)
    }

    fn atom(&mut self) -> Option<f64> {
        match self.peek()? {
            Tok::Num(n) => {
                self.at += 1;
                Some(n)
            }
            Tok::Open => {
                self.at += 1;
                let v = self.sum()?;
                (self.peek() == Some(Tok::Close)).then_some(())?;
                self.at += 1;
                Some(v)
            }
            _ => None,
        }
    }
}

fn arithmetic(text: &str) -> Option<f64> {
    let mut p = Parser {
        toks: tokens(text)?,
        at: 0,
        binary: 0,
    };
    let v = p.sum()?;
    (p.at == p.toks.len() && p.binary > 0 && v.is_finite()).then_some(v)
}

// Unit conversions: "<number> <unit> to|in <unit>".

#[derive(Clone, Copy, PartialEq, Eq)]
enum Dim {
    Length,
    Mass,
    Volume,
    Time,
    Data,
    Speed,
    Temperature,
}

/// Aliases, the symbol shown, the dimension and the factor to the base unit.
const UNITS: &[(&[&str], &str, Dim, f64)] = &[
    (
        &["m", "meter", "meters", "metre", "metres"],
        "m",
        Dim::Length,
        1.0,
    ),
    (
        &["km", "kilometer", "kilometers", "kilometre", "kilometres"],
        "km",
        Dim::Length,
        1000.0,
    ),
    (
        &["cm", "centimeter", "centimeters"],
        "cm",
        Dim::Length,
        0.01,
    ),
    (
        &["mm", "millimeter", "millimeters"],
        "mm",
        Dim::Length,
        0.001,
    ),
    (&["mi", "mile", "miles"], "mi", Dim::Length, 1609.344),
    (&["yd", "yard", "yards"], "yd", Dim::Length, 0.9144),
    (&["ft", "foot", "feet"], "ft", Dim::Length, 0.3048),
    (&["in", "inch", "inches"], "in", Dim::Length, 0.0254),
    (&["nmi"], "nmi", Dim::Length, 1852.0),
    (&["kg", "kilogram", "kilograms"], "kg", Dim::Mass, 1.0),
    (&["g", "gram", "grams"], "g", Dim::Mass, 0.001),
    (
        &["mg", "milligram", "milligrams"],
        "mg",
        Dim::Mass,
        0.000_001,
    ),
    (
        &["lb", "lbs", "pound", "pounds"],
        "lb",
        Dim::Mass,
        0.453_592_37,
    ),
    (
        &["oz", "ounce", "ounces"],
        "oz",
        Dim::Mass,
        0.028_349_523_125,
    ),
    (&["t", "tonne", "tonnes"], "t", Dim::Mass, 1000.0),
    (&["st", "stone"], "st", Dim::Mass, 6.350_293_18),
    (
        &["l", "liter", "liters", "litre", "litres"],
        "L",
        Dim::Volume,
        1.0,
    ),
    (
        &["ml", "milliliter", "milliliters"],
        "mL",
        Dim::Volume,
        0.001,
    ),
    (
        &["gal", "gallon", "gallons"],
        "gal",
        Dim::Volume,
        3.785_411_784,
    ),
    (&["qt", "quart", "quarts"], "qt", Dim::Volume, 0.946_352_946),
    (&["pt", "pint", "pints"], "pt", Dim::Volume, 0.473_176_473),
    (&["cup", "cups"], "cups", Dim::Volume, 0.236_588_236_5),
    (&["floz"], "fl oz", Dim::Volume, 0.029_573_529_562_5),
    (&["tbsp"], "tbsp", Dim::Volume, 0.014_786_764_781_25),
    (&["tsp"], "tsp", Dim::Volume, 0.004_928_921_593_75),
    (&["s", "sec", "second", "seconds"], "s", Dim::Time, 1.0),
    (&["min", "minute", "minutes"], "min", Dim::Time, 60.0),
    (&["h", "hr", "hour", "hours"], "h", Dim::Time, 3600.0),
    (&["day", "days"], "days", Dim::Time, 86_400.0),
    (&["week", "weeks"], "weeks", Dim::Time, 604_800.0),
    (&["b", "byte", "bytes"], "B", Dim::Data, 1.0),
    (&["kb"], "KB", Dim::Data, 1e3),
    (&["mb"], "MB", Dim::Data, 1e6),
    (&["gb"], "GB", Dim::Data, 1e9),
    (&["tb"], "TB", Dim::Data, 1e12),
    (&["kib"], "KiB", Dim::Data, 1024.0),
    (&["mib"], "MiB", Dim::Data, 1_048_576.0),
    (&["gib"], "GiB", Dim::Data, 1_073_741_824.0),
    (&["tib"], "TiB", Dim::Data, 1_099_511_627_776.0),
    (&["m/s"], "m/s", Dim::Speed, 1.0),
    (&["km/h", "kph"], "km/h", Dim::Speed, 1.0 / 3.6),
    (&["mph"], "mph", Dim::Speed, 0.447_04),
    (&["kn", "knot", "knots"], "kn", Dim::Speed, 1852.0 / 3600.0),
    (&["c", "°c", "celsius"], "°C", Dim::Temperature, 0.0),
    (&["f", "°f", "fahrenheit"], "°F", Dim::Temperature, 0.0),
    (&["k", "kelvin"], "K", Dim::Temperature, 0.0),
];

fn unit(name: &str) -> Option<(&'static str, Dim, f64)> {
    let name = name.to_lowercase();
    UNITS
        .iter()
        .find(|(aliases, ..)| aliases.contains(&name.as_str()))
        .map(|(_, symbol, dim, factor)| (*symbol, *dim, *factor))
}

fn to_kelvin(symbol: &str, v: f64) -> f64 {
    match symbol {
        "°C" => v + 273.15,
        "°F" => (v - 32.0) * 5.0 / 9.0 + 273.15,
        _ => v,
    }
}

fn from_kelvin(symbol: &str, k: f64) -> f64 {
    match symbol {
        "°C" => k - 273.15,
        "°F" => (k - 273.15) * 9.0 / 5.0 + 32.0,
        _ => k,
    }
}

fn units(text: &str) -> Option<String> {
    let words: Vec<&str> = text.split_whitespace().collect();
    let (amount, from, target) = match words.as_slice() {
        [n, from, "to" | "in", to] => (*n, *from, *to),
        [joined, "to" | "in", to] => {
            let split = joined.find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-'))?;
            let (n, from) = joined.split_at(split);
            (n, from, *to)
        }
        _ => return None,
    };
    let value: f64 = amount.parse().ok()?;
    let (from_sym, from_dim, from_f) = unit(from)?;
    let (to_sym, to_dim, to_f) = unit(target)?;
    if from_dim != to_dim {
        return None;
    }
    let out = if from_dim == Dim::Temperature {
        from_kelvin(to_sym, to_kelvin(from_sym, value))
    } else {
        value * from_f / to_f
    };
    out.is_finite()
        .then(|| format!("{} {to_sym}", format_number(out)))
}

// Time zones: "<time> <zone> in|to <zone>".

/// Abbreviations, as the fixed offsets they name (hours, minutes).
const ABBREVIATIONS: &[(&str, i8, i8)] = &[
    ("UTC", 0, 0),
    ("GMT", 0, 0),
    ("PST", -8, 0),
    ("PDT", -7, 0),
    ("MST", -7, 0),
    ("MDT", -6, 0),
    ("CST", -6, 0),
    ("CDT", -5, 0),
    ("EST", -5, 0),
    ("EDT", -4, 0),
    ("BST", 1, 0),
    ("CET", 1, 0),
    ("CEST", 2, 0),
    ("EET", 2, 0),
    ("EEST", 3, 0),
    ("IST", 5, 30),
    ("JST", 9, 0),
    ("KST", 9, 0),
    ("AEST", 10, 0),
    ("AEDT", 11, 0),
];

/// Cities, as the IANA zone they are in.
const CITIES: &[(&str, &str)] = &[
    ("London", "Europe/London"),
    ("Paris", "Europe/Paris"),
    ("Berlin", "Europe/Berlin"),
    ("Madrid", "Europe/Madrid"),
    ("Rome", "Europe/Rome"),
    ("Amsterdam", "Europe/Amsterdam"),
    ("Stockholm", "Europe/Stockholm"),
    ("Moscow", "Europe/Moscow"),
    ("Istanbul", "Europe/Istanbul"),
    ("Dubai", "Asia/Dubai"),
    ("Delhi", "Asia/Kolkata"),
    ("Mumbai", "Asia/Kolkata"),
    ("Bangalore", "Asia/Kolkata"),
    ("Singapore", "Asia/Singapore"),
    ("Hong Kong", "Asia/Hong_Kong"),
    ("Shanghai", "Asia/Shanghai"),
    ("Beijing", "Asia/Shanghai"),
    ("Tokyo", "Asia/Tokyo"),
    ("Seoul", "Asia/Seoul"),
    ("Sydney", "Australia/Sydney"),
    ("Melbourne", "Australia/Melbourne"),
    ("Auckland", "Pacific/Auckland"),
    ("New York", "America/New_York"),
    ("Boston", "America/New_York"),
    ("Toronto", "America/Toronto"),
    ("Chicago", "America/Chicago"),
    ("Denver", "America/Denver"),
    ("Los Angeles", "America/Los_Angeles"),
    ("San Francisco", "America/Los_Angeles"),
    ("Seattle", "America/Los_Angeles"),
    ("Mexico City", "America/Mexico_City"),
    ("São Paulo", "America/Sao_Paulo"),
    ("Sao Paulo", "America/Sao_Paulo"),
];

/// A zone by abbreviation, city or IANA name, with the label shown.
fn zone(name: &str) -> Option<(TimeZone, String)> {
    let name = name.trim();
    if let Some((abbr, h, m)) = ABBREVIATIONS
        .iter()
        .find(|(a, ..)| a.eq_ignore_ascii_case(name))
    {
        let secs = i32::from(*h) * 3600 + i32::from(*m) * 60;
        let offset = tz::Offset::from_seconds(secs).ok()?;
        return Some((TimeZone::fixed(offset), (*abbr).to_owned()));
    }
    if let Some((city, iana)) = CITIES.iter().find(|(c, _)| c.eq_ignore_ascii_case(name)) {
        return Some((TimeZone::get(iana).ok()?, (*city).to_owned()));
    }
    if name.contains('/') {
        return Some((TimeZone::get(name).ok()?, name.to_owned()));
    }
    None
}

/// `3pm`, `3:30pm`, `3 pm` (joined before), `15:00`.
fn clock(text: &str) -> Option<Time> {
    let t = text.to_lowercase();
    let (body, pm) = if let Some(b) = t.strip_suffix("pm") {
        (b, Some(true))
    } else if let Some(b) = t.strip_suffix("am") {
        (b, Some(false))
    } else {
        (t.as_str(), None)
    };
    let (h, m) = match body.split_once(':') {
        Some((h, m)) => (h.parse::<i8>().ok()?, m.parse::<i8>().ok()?),
        None if pm.is_some() => (body.parse::<i8>().ok()?, 0),
        None => return None,
    };
    let h = match pm {
        Some(pm) if (1..=12).contains(&h) => (h % 12) + if pm { 12 } else { 0 },
        Some(_) => return None,
        None => h,
    };
    Time::new(h, m, 0, 0).ok()
}

fn time_zone(text: &str, now: &jiff::Zoned) -> Option<String> {
    let lower = text.to_lowercase();
    let at = lower.find(" in ").or_else(|| lower.find(" to "))?;
    let (left, target) = (&text[..at], &text[at + 4..]);
    let mut words: Vec<&str> = left.split_whitespace().collect();
    // "3 pm PST": join the meridiem onto the number.
    let joined;
    if words.len() == 3 && matches!(words[1].to_lowercase().as_str(), "am" | "pm") {
        joined = format!("{}{}", words[0], words[1]);
        words = vec![joined.as_str(), words[2]];
    }
    let [time, from] = words.as_slice() else {
        return None;
    };
    let time = clock(time)?;
    let (from_tz, _) = zone(from)?;
    let (to_tz, label) = zone(target)?;
    let day = now.with_time_zone(from_tz.clone()).date();
    let start = day.to_datetime(time).to_zoned(from_tz).ok()?;
    let there = start.with_time_zone(to_tz);
    let shift = there.date().since(day).ok()?.get_days();
    let suffix = match shift {
        0 => String::new(),
        1 => " (+1 day)".into(),
        -1 => " (-1 day)".into(),
        n => format!(" ({n:+} days)"),
    };
    Some(format!(
        "{:02}:{:02} {label}{suffix}",
        there.hour(),
        there.minute()
    ))
}

// Dates: "days until <date>", "days since <date>", "<date> to <date>".

fn days(n: i32) -> String {
    if n.abs() == 1 {
        format!("{n} day")
    } else {
        format!("{n} days")
    }
}

fn dates(text: &str, now: &jiff::Zoned) -> Option<String> {
    let lower = text.to_lowercase();
    let today = now.date();
    let between = |a: Date, b: Date| a.until(b).ok().map(|s| s.get_days());
    if let Some(rest) = lower.strip_prefix("days until ") {
        let d: Date = rest.trim().parse().ok()?;
        return between(today, d).map(days);
    }
    if let Some(rest) = lower.strip_prefix("days since ") {
        let d: Date = rest.trim().parse().ok()?;
        return between(d, today).map(days);
    }
    let (a, b) = lower.split_once(" to ")?;
    let a: Date = a.trim().parse().ok()?;
    let b: Date = b.trim().parse().ok()?;
    between(a, b).map(days)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_keep_ten_significant_digits() {
        assert_eq!(format_number(1.0 / 3.0), "0.3333333333");
        assert_eq!(format_number(1234.5678), "1234.5678");
        assert_eq!(format_number(-2.0), "-2");
    }

    #[test]
    fn a_lone_number_is_not_a_calculation() {
        assert_eq!(arithmetic("42"), None);
        assert_eq!(arithmetic("-42"), None);
        assert_eq!(arithmetic("1/0"), None);
    }
}
