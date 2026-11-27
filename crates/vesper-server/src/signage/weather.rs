//! The weather from Open-Meteo (no key; CC BY 4.0: the credit shows wherever the weather does)
//! and its geocoding (a place found by name). Read with the IDN library's HTTP client.

use protocol::{SignagePlace, SignageWeather};
use serde_json::Value;
use std::time::Duration;

/// The credit shown with the weather.
pub const CREDIT: &str = "Weather data by Open-Meteo.com";
/// Open-Meteo's forecast API.
pub const FORECAST: &str = "https://api.open-meteo.com/v1/forecast";
/// Open-Meteo's geocoding API.
pub const GEOCODING: &str = "https://geocoding-api.open-meteo.com/v1/search";
/// How often the forecast is read.
pub const REFRESH: Duration = Duration::from_secs(30 * 60);
/// A forecast older than this isn't shown.
pub const STALE: Duration = Duration::from_secs(6 * 3600);

fn get(url: &str) -> Result<Value, String> {
    let body = idn_open_protocol::web::httpc::get_up_to(url, Duration::from_secs(10), 1 << 20)
        .map_err(|e| e.to_string())?;
    serde_json::from_slice(&body).map_err(|e| format!("not JSON: {e}"))
}

/// Encodes a query value.
fn enc(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// The forecast's address for a place.
pub fn forecast_url(base: &str, place: &SignagePlace, fahrenheit: bool) -> String {
    format!(
        "{base}?latitude={:.4}&longitude={:.4}&current=temperature_2m,weather_code\
         &daily=temperature_2m_max,temperature_2m_min&timezone=auto&forecast_days=1{}",
        place.latitude,
        place.longitude,
        if fahrenheit {
            "&temperature_unit=fahrenheit"
        } else {
            ""
        }
    )
}

/// Reads the forecast now (blocking).
pub fn read(
    base: &str,
    place: &SignagePlace,
    fahrenheit: bool,
    now_ms: u64,
) -> Result<SignageWeather, String> {
    parse_forecast(&get(&forecast_url(base, place, fahrenheit))?, now_ms)
}

/// A forecast answer as the weather now.
pub fn parse_forecast(v: &Value, now_ms: u64) -> Result<SignageWeather, String> {
    let num = |v: &Value| v.as_f64().ok_or("a number is missing".to_string());
    let cur = v.get("current").ok_or("no current weather")?;
    let daily = v.get("daily").ok_or("no daily forecast")?;
    let first = |k: &str| {
        daily
            .get(k)
            .and_then(|a| a.get(0))
            .ok_or(format!("no {k}"))
            .and_then(num)
    };
    let code = cur
        .get("weather_code")
        .and_then(Value::as_i64)
        .unwrap_or(-1);
    Ok(SignageWeather {
        temperature: num(cur.get("temperature_2m").unwrap_or(&Value::Null))?,
        high: first("temperature_2m_max")?,
        low: first("temperature_2m_min")?,
        summary: summary(code).into(),
        read_at: now_ms,
        credit: CREDIT.into(),
    })
}

/// Places found by name (blocking).
pub fn places(base: &str, q: &str) -> Result<Vec<SignagePlace>, String> {
    let url = format!(
        "{base}?name={}&count=8&language=en&format=json",
        enc(q.trim())
    );
    Ok(parse_places(&get(&url)?))
}

/// A geocoding answer as places ("Name, Region, Country").
pub fn parse_places(v: &Value) -> Vec<SignagePlace> {
    let Some(list) = v.get("results").and_then(Value::as_array) else {
        return Vec::new();
    };
    list.iter()
        .filter_map(|r| {
            let s = |k: &str| r.get(k).and_then(Value::as_str).filter(|s| !s.is_empty());
            let mut name = s("name")?.to_string();
            for k in ["admin1", "country"] {
                if let Some(x) = s(k) {
                    name.push_str(", ");
                    name.push_str(x);
                }
            }
            Some(SignagePlace {
                name,
                latitude: r.get("latitude")?.as_f64()?,
                longitude: r.get("longitude")?.as_f64()?,
            })
        })
        .collect()
}

/// A WMO weather code in words.
pub fn summary(code: i64) -> &'static str {
    match code {
        0 => "Clear",
        1 => "Mainly clear",
        2 => "Partly cloudy",
        3 => "Overcast",
        45 | 48 => "Fog",
        51 | 53 | 55 | 56 | 57 => "Drizzle",
        61 | 63 | 65 | 66 | 67 => "Rain",
        71 | 73 | 75 | 77 => "Snow",
        80..=82 => "Showers",
        85 | 86 => "Snow showers",
        95 | 96 | 99 => "Thunderstorm",
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_forecast_reads_as_the_weather_now() {
        let v = json!({
            "current": {"time": "2026-11-25T10:00", "temperature_2m": 7.4, "weather_code": 2},
            "daily": {"temperature_2m_max": [9.1], "temperature_2m_min": [1.5]}
        });
        let w = parse_forecast(&v, 42).unwrap();
        assert_eq!((w.temperature, w.high, w.low), (7.4, 9.1, 1.5));
        assert_eq!(w.summary, "Partly cloudy");
        assert_eq!((w.read_at, w.credit.as_str()), (42, CREDIT));
        assert!(parse_forecast(&json!({"current": {}}), 0).is_err());
    }

    #[test]
    fn places_and_addresses() {
        let v = json!({"results": [
            {"name": "Springfield", "admin1": "Illinois", "country": "United States",
             "latitude": 39.8, "longitude": -89.64},
            {"name": "Nowhere"}
        ]});
        let p = parse_places(&v);
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].name, "Springfield, Illinois, United States");
        assert!(parse_places(&json!({})).is_empty());
        let url = forecast_url(FORECAST, &p[0], true);
        assert!(url.contains("latitude=39.8000&longitude=-89.6400"), "{url}");
        assert!(url.ends_with("&temperature_unit=fahrenheit"));
        assert_eq!(enc("Saint-Lô ok"), "Saint-L%C3%B4%20ok");
    }
}
