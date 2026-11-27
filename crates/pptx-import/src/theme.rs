//! The theme (colour scheme, font scheme) and DrawingML colours with their modifiers.

use crate::opc::{attr, attr_i64, child, path};
use model::Color;
use roxmltree::Node;
use std::collections::HashMap;

/// A theme's colours and fonts, and the master's colour map.
#[derive(Clone, Debug)]
pub struct Palette {
    /// `dk1`, `lt1`, `dk2`, `lt2`, `accent1`…`accent6`, `hlink`, `folHlink`.
    pub colors: HashMap<String, Color>,
    /// `bg1` → `lt1`, `tx1` → `dk1`, … (the master's `p:clrMap`).
    pub map: HashMap<String, String>,
    /// The heading font (`+mj-lt`).
    pub major: String,
    /// The body font (`+mn-lt`).
    pub minor: String,
}

impl Default for Palette {
    /// Office's default theme.
    fn default() -> Self {
        let c = |s: &str| hex(s).unwrap_or(Color::BLACK);
        let colors = [
            ("dk1", c("000000")),
            ("lt1", c("FFFFFF")),
            ("dk2", c("44546A")),
            ("lt2", c("E7E6E6")),
            ("accent1", c("4472C4")),
            ("accent2", c("ED7D31")),
            ("accent3", c("A5A5A5")),
            ("accent4", c("FFC000")),
            ("accent5", c("5B9BD5")),
            ("accent6", c("70AD47")),
            ("hlink", c("0563C1")),
            ("folHlink", c("954F72")),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
        Palette {
            colors,
            map: default_map(),
            major: "Calibri Light".into(),
            minor: "Calibri".into(),
        }
    }
}

fn default_map() -> HashMap<String, String> {
    [
        ("bg1", "lt1"),
        ("tx1", "dk1"),
        ("bg2", "lt2"),
        ("tx2", "dk2"),
    ]
    .into_iter()
    .map(|(a, b)| (a.to_string(), b.to_string()))
    .collect()
}

impl Palette {
    /// Reads `a:theme` (colour and font scheme); unset parts keep Office's defaults.
    pub fn read_theme(&mut self, theme: Node) {
        if let Some(cs) = path(theme, &["themeElements", "clrScheme"]) {
            for c in cs.children().filter(|n| n.is_element()) {
                let name = c.tag_name().name().to_string();
                if let Some(col) = self.base_color(c) {
                    self.colors.insert(name, col);
                }
            }
        }
        if let Some(fs) = path(theme, &["themeElements", "fontScheme"]) {
            if let Some(f) = path(fs, &["majorFont", "latin"]).and_then(|n| attr(n, "typeface"))
                && !f.is_empty()
            {
                self.major = f.to_string();
            }
            if let Some(f) = path(fs, &["minorFont", "latin"]).and_then(|n| attr(n, "typeface"))
                && !f.is_empty()
            {
                self.minor = f.to_string();
            }
        }
    }

    /// Reads a master's `p:clrMap`.
    pub fn read_map(&mut self, clr_map: Node) {
        for a in clr_map.attributes() {
            self.map.insert(a.name().to_string(), a.value().to_string());
        }
    }

    /// A scheme colour by name (`tx1` through the colour map, `accent1` directly).
    pub fn scheme(&self, name: &str) -> Option<Color> {
        let key = self.map.get(name).map(String::as_str).unwrap_or(name);
        self.colors.get(key).copied()
    }

    /// The font a typeface names: `+mj-lt` / `+mn-lt` are the theme's.
    pub fn font(&self, typeface: &str) -> String {
        match typeface {
            t if t.starts_with("+mj") => self.major.clone(),
            t if t.starts_with("+mn") => self.minor.clone(),
            t => t.to_string(),
        }
    }

    /// The colour inside a fill-like element (`a:solidFill`, a `clrScheme` entry, `a:gs`…):
    /// its first colour child with the modifiers applied.
    pub fn color_in(&self, n: Node) -> Option<Color> {
        n.children()
            .filter(|c| c.is_element())
            .find_map(|c| self.color(c))
    }

    /// A colour element (`a:srgbClr`, `a:schemeClr`, `a:sysClr`, `a:prstClr`, `a:scrgbClr`,
    /// `a:hslClr`) with its modifiers.
    pub fn color(&self, c: Node) -> Option<Color> {
        let base = match c.tag_name().name() {
            "schemeClr" => self.scheme(attr(c, "val")?)?,
            _ => self.base_color(c)?,
        };
        Some(modify(base, c))
    }

    fn base_color(&self, c: Node) -> Option<Color> {
        // A clrScheme entry (`a:dk1`) holds the colour element.
        let c = match c.tag_name().name() {
            "srgbClr" | "sysClr" | "prstClr" | "scrgbClr" | "hslClr" | "schemeClr" => c,
            _ => c.children().find(|n| n.is_element())?,
        };
        let col = match c.tag_name().name() {
            "srgbClr" => hex(attr(c, "val")?)?,
            "sysClr" => attr(c, "lastClr")
                .and_then(hex)
                .or_else(|| match attr(c, "val")? {
                    "window" => Some(Color::WHITE),
                    _ => Some(Color::BLACK),
                })?,
            "prstClr" => preset(attr(c, "val")?)?,
            "scrgbClr" => {
                let p = |k| attr_i64(c, k).map(|v| linear_to_srgb(v as f32 / 100_000.0));
                Color::rgb(p("r")?, p("g")?, p("b")?)
            }
            "hslClr" => {
                let h = attr_i64(c, "hue")? as f32 / 60_000.0 / 360.0;
                let s = attr_i64(c, "sat")? as f32 / 100_000.0;
                let l = attr_i64(c, "lum")? as f32 / 100_000.0;
                let (r, g, b) = hsl_to_rgb(h, s, l);
                Color::rgb(r, g, b)
            }
            "schemeClr" => self.scheme(attr(c, "val")?)?,
            _ => return None,
        };
        Some(modify(col, c))
    }
}

/// `RRGGBB` → colour.
pub fn hex(s: &str) -> Option<Color> {
    let s = s.trim();
    if s.len() != 6 || !s.is_ascii() {
        return None;
    }
    let v = |i: usize| u8::from_str_radix(&s[i..i + 2], 16).ok();
    Some(Color::from_rgb8(v(0)?, v(2)?, v(4)?))
}

fn preset(name: &str) -> Option<Color> {
    let h = match name {
        "black" => "000000",
        "white" => "FFFFFF",
        "red" => "FF0000",
        "green" => "008000",
        "lime" => "00FF00",
        "blue" => "0000FF",
        "yellow" => "FFFF00",
        "gray" | "grey" => "808080",
        "darkGray" | "dkGray" => "A9A9A9",
        "lightGray" | "ltGray" => "D3D3D3",
        "orange" => "FFA500",
        "purple" => "800080",
        "navy" => "000080",
        "silver" => "C0C0C0",
        "maroon" => "800000",
        "teal" => "008080",
        "cyan" | "aqua" => "00FFFF",
        "magenta" | "fuchsia" => "FF00FF",
        _ => return None,
    };
    hex(h)
}

fn linear_to_srgb(v: f32) -> f32 {
    let v = v.clamp(0.0, 1.0);
    if v <= 0.003_130_8 {
        v * 12.92
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    }
}

fn rgb_to_hsl(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    if (max - min).abs() < f32::EPSILON {
        return (0.0, 0.0, l);
    }
    let d = max - min;
    let s = if l > 0.5 {
        d / (2.0 - max - min)
    } else {
        d / (max + min)
    };
    let h = if max == r {
        (g - b) / d + if g < b { 6.0 } else { 0.0 }
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (h / 6.0, s, l)
}

fn hsl_to_rgb(h: f32, s: f32, l: f32) -> (f32, f32, f32) {
    if s <= 0.0 {
        return (l, l, l);
    }
    let q = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let p = 2.0 * l - q;
    let f = |mut t: f32| {
        if t < 0.0 {
            t += 1.0;
        }
        if t > 1.0 {
            t -= 1.0;
        }
        if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        }
    };
    (f(h + 1.0 / 3.0), f(h), f(h - 1.0 / 3.0))
}

/// Applies a colour element's modifiers (`lumMod`, `lumOff`, `tint`, `shade`, `satMod`,
/// `alpha`), in document order.
fn modify(mut c: Color, el: Node) -> Color {
    for m in el.children().filter(|n| n.is_element()) {
        let Some(v) = attr_i64(m, "val").map(|v| v as f32 / 100_000.0) else {
            continue;
        };
        match m.tag_name().name() {
            "alpha" => c.a = v.clamp(0.0, 1.0),
            "alphaMod" => c.a = (c.a * v).clamp(0.0, 1.0),
            "tint" => {
                let t = |x: f32| x * v + (1.0 - v);
                c = Color::rgba(t(c.r), t(c.g), t(c.b), c.a);
            }
            "shade" => c = Color::rgba(c.r * v, c.g * v, c.b * v, c.a),
            "lumMod" | "lumOff" | "satMod" => {
                let (h, mut s, mut l) = rgb_to_hsl(c.r, c.g, c.b);
                match m.tag_name().name() {
                    "lumMod" => l *= v,
                    "lumOff" => l += v,
                    _ => s *= v,
                }
                let (r, g, b) = hsl_to_rgb(h, s.clamp(0.0, 1.0), l.clamp(0.0, 1.0));
                c = Color::rgba(r, g, b, c.a);
            }
            _ => {}
        }
    }
    c
}

/// The `a:solidFill` (or other colour container) child of `n` as a colour.
pub fn solid_fill(p: &Palette, n: Node) -> Option<Color> {
    child(n, "solidFill").and_then(|f| p.color_in(f))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with<R>(xml: &str, f: impl FnOnce(Node) -> R) -> R {
        let doc = roxmltree::Document::parse(xml).unwrap();
        f(doc.root_element())
    }

    #[test]
    fn scheme_colours_go_through_the_map_and_modifiers_apply() {
        let p = Palette::default();
        let c = with(
            r#"<a:solidFill xmlns:a="x"><a:schemeClr val="tx1"/></a:solidFill>"#,
            |n| p.color_in(n),
        )
        .unwrap();
        assert_eq!(c, Color::BLACK);
        let c = with(
            r#"<a:solidFill xmlns:a="x"><a:srgbClr val="FF0000"><a:alpha val="50000"/></a:srgbClr></a:solidFill>"#,
            |n| p.color_in(n),
        )
        .unwrap();
        assert_eq!((c.r, c.g, c.b, c.a), (1.0, 0.0, 0.0, 0.5));
        // White at 50 % luminance is mid grey.
        let c = with(
            r#"<a:solidFill xmlns:a="x"><a:schemeClr val="bg1"><a:lumMod val="50000"/></a:schemeClr></a:solidFill>"#,
            |n| p.color_in(n),
        )
        .unwrap();
        assert!((c.r - 0.5).abs() < 0.01 && (c.g - 0.5).abs() < 0.01);
    }

    #[test]
    fn reads_a_theme() {
        let mut p = Palette::default();
        with(
            r#"<a:theme xmlns:a="x"><a:themeElements>
            <a:clrScheme name="t"><a:dk1><a:sysClr val="windowText" lastClr="111111"/></a:dk1>
            <a:accent1><a:srgbClr val="00FF00"/></a:accent1></a:clrScheme>
            <a:fontScheme name="f"><a:majorFont><a:latin typeface="Georgia"/></a:majorFont>
            <a:minorFont><a:latin typeface="Verdana"/></a:minorFont></a:fontScheme>
            </a:themeElements></a:theme>"#,
            |n| p.read_theme(n),
        );
        assert_eq!(p.scheme("tx1"), hex("111111"));
        assert_eq!(p.scheme("accent1"), hex("00FF00"));
        assert_eq!(p.font("+mj-lt"), "Georgia");
        assert_eq!(p.font("+mn-lt"), "Verdana");
        assert_eq!(p.font("Arial"), "Arial");
    }
}
