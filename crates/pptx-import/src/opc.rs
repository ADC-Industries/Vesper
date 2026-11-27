//! OOXML package plumbing: part relationships, relative part names and XML helpers.

use crate::zip::Archive;
use anyhow::{Result, bail};
use roxmltree::{Document, Node, ParsingOptions};
use std::collections::HashMap;

/// Deepest element nesting a part may have. Real parts nest about 20 deep (groups add a few
/// levels each); the XML parser and our own shape walkers recurse once per level, so a
/// crafted part nested thousands deep would overflow the stack and take the server down.
pub const MAX_DEPTH: usize = 256;
/// Most XML nodes one part may have (memory: a node costs tens of bytes).
pub const MAX_NODES: u32 = 4_000_000;

/// Parses a part of an untrusted deck: refused when nested deeper than [`MAX_DEPTH`] (checked
/// first, without recursion) or larger than [`MAX_NODES`] nodes. No DTDs.
pub fn xml(text: &str) -> Result<Document<'_>> {
    let depth = nesting(text);
    if depth > MAX_DEPTH {
        bail!("the XML is nested too deeply (more than {MAX_DEPTH} levels)");
    }
    Ok(Document::parse_with_options(
        text,
        ParsingOptions {
            allow_dtd: false,
            nodes_limit: MAX_NODES,
        },
    )?)
}

/// The deepest element nesting of `text`, counted without recursion (never less than the
/// real depth: a malformed part may count deeper than it is, which only refuses it sooner).
pub fn nesting(text: &str) -> usize {
    let b = text.as_bytes();
    let find = |from: usize, pat: &[u8]| {
        b.get(from..)
            .and_then(|rest| rest.windows(pat.len()).position(|w| w == pat))
            .map(|i| from + i + pat.len())
            .unwrap_or(b.len())
    };
    let (mut depth, mut max, mut i) = (0usize, 0usize, 0usize);
    while i < b.len() {
        if b[i] != b'<' {
            i += 1;
            continue;
        }
        match b.get(i + 1) {
            Some(b'/') => {
                depth = depth.saturating_sub(1);
                i = find(i, b">");
            }
            Some(b'?') => i = find(i, b"?>"),
            Some(b'!') if b[i..].starts_with(b"<!--") => i = find(i + 4, b"-->"),
            Some(b'!') if b[i..].starts_with(b"<![CDATA[") => i = find(i, b"]]>"),
            Some(b'!') => i = find(i, b">"),
            _ => {
                // A start tag: to its `>`, past quoted attribute values.
                let mut j = i + 1;
                let mut quote = None;
                while j < b.len() {
                    match (quote, b[j]) {
                        (None, b'"' | b'\'') => quote = Some(b[j]),
                        (Some(q), c) if c == q => quote = None,
                        (None, b'>') => break,
                        _ => {}
                    }
                    j += 1;
                }
                // An empty element (`<a/>`) is one level down but holds nothing.
                max = max.max(depth + 1);
                if b.get(j.wrapping_sub(1)) != Some(&b'/') || j >= b.len() {
                    depth += 1;
                }
                i = j + 1;
            }
        }
    }
    max
}

/// One relationship of a part.
#[derive(Clone, Debug)]
pub struct Rel {
    /// The relationship type's last path segment (`slide`, `slideLayout`, `image`, `notesSlide`).
    pub kind: String,
    /// The target part (resolved, no leading `/`), or the external URL.
    pub target: String,
    /// `TargetMode="External"`.
    pub external: bool,
}

/// Relationships of a part by ID.
#[derive(Clone, Debug, Default)]
pub struct Rels(pub HashMap<String, Rel>);

impl Rels {
    /// The relationships of `part` (`ppt/slides/slide1.xml` → `ppt/slides/_rels/slide1.xml.rels`).
    pub fn of(zip: &Archive, part: &str) -> Rels {
        let (dir, file) = split(part);
        let path = if dir.is_empty() {
            format!("_rels/{file}.rels")
        } else {
            format!("{dir}/_rels/{file}.rels")
        };
        let Ok(text) = zip.text(&path) else {
            return Rels::default();
        };
        let Ok(doc) = xml(&text) else {
            return Rels::default();
        };
        let mut out = HashMap::new();
        for r in doc
            .root_element()
            .children()
            .filter(|n| is(n, "Relationship"))
        {
            let (Some(id), Some(target)) = (r.attribute("Id"), r.attribute("Target")) else {
                continue;
            };
            let kind = r
                .attribute("Type")
                .and_then(|t| t.rsplit('/').next())
                .unwrap_or_default()
                .to_string();
            let external = r.attribute("TargetMode") == Some("External");
            let target = if external {
                target.to_string()
            } else {
                resolve(dir, target)
            };
            out.insert(
                id.to_string(),
                Rel {
                    kind,
                    target,
                    external,
                },
            );
        }
        Rels(out)
    }

    /// The internal target of relationship `id`.
    pub fn target(&self, id: &str) -> Option<&str> {
        self.0
            .get(id)
            .filter(|r| !r.external)
            .map(|r| r.target.as_str())
    }

    /// The first internal target of kind `kind`.
    pub fn first(&self, kind: &str) -> Option<&str> {
        let mut v: Vec<(&String, &Rel)> = self
            .0
            .iter()
            .filter(|(_, r)| r.kind == kind && !r.external)
            .collect();
        v.sort_by(|a, b| a.0.cmp(b.0));
        v.first().map(|(_, r)| r.target.as_str())
    }
}

/// `(dir, file)` of a part name.
pub fn split(part: &str) -> (&str, &str) {
    match part.rfind('/') {
        Some(i) => (&part[..i], &part[i + 1..]),
        None => ("", part),
    }
}

/// `target` relative to the folder `dir` (or absolute from the package root with `/`).
pub fn resolve(dir: &str, target: &str) -> String {
    let mut parts: Vec<&str> = if target.starts_with('/') {
        Vec::new()
    } else {
        dir.split('/').filter(|s| !s.is_empty()).collect()
    };
    for seg in target.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    let joined = parts.join("/");
    // Percent-encoded names (`image%201.png`).
    if joined.contains('%') {
        percent_decode(&joined)
    } else {
        joined
    }
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let hex = |c: u8| (c as char).to_digit(16);
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2]))
        {
            out.push((h * 16 + l) as u8);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// True when `n` is an element with local name `name`.
pub fn is(n: &Node, name: &str) -> bool {
    n.is_element() && n.tag_name().name() == name
}

/// The first child element named `name`.
pub fn child<'a, 'i>(n: Node<'a, 'i>, name: &str) -> Option<Node<'a, 'i>> {
    n.children().find(|c| is(c, name))
}

/// Child elements named `name`.
pub fn children<'a, 'i>(n: Node<'a, 'i>, name: &'a str) -> impl Iterator<Item = Node<'a, 'i>> {
    n.children().filter(move |c| is(c, name))
}

/// The element at `path` (child names) below `n`.
pub fn path<'a, 'i>(n: Node<'a, 'i>, path: &[&str]) -> Option<Node<'a, 'i>> {
    let mut cur = n;
    for p in path {
        cur = child(cur, p)?;
    }
    Some(cur)
}

/// An attribute by its local name (any namespace).
pub fn attr<'a>(n: Node<'a, '_>, name: &str) -> Option<&'a str> {
    n.attributes().find(|a| a.name() == name).map(|a| a.value())
}

/// An integer attribute.
pub fn attr_i64(n: Node, name: &str) -> Option<i64> {
    attr(n, name).and_then(|v| v.trim().parse().ok())
}

/// A boolean attribute (`1`/`true`).
pub fn attr_bool(n: Node, name: &str) -> Option<bool> {
    attr(n, name).map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_relative_targets() {
        assert_eq!(
            resolve("ppt/slides", "../media/image1.png"),
            "ppt/media/image1.png"
        );
        assert_eq!(resolve("ppt", "slides/slide1.xml"), "ppt/slides/slide1.xml");
        assert_eq!(resolve("ppt/slides", "/ppt/x.xml"), "ppt/x.xml");
        assert_eq!(resolve("", "ppt/presentation.xml"), "ppt/presentation.xml");
        assert_eq!(resolve("ppt/media", "a%20b.png"), "ppt/media/a b.png");
        assert_eq!(resolve("ppt/media", "é%é%"), "ppt/media/é%é%");
        assert_eq!(split("ppt/slides/slide1.xml"), ("ppt/slides", "slide1.xml"));
    }

    #[test]
    fn counts_nesting_without_recursion() {
        assert_eq!(nesting("<a/>"), 1);
        assert_eq!(
            nesting("<?xml version='1.0'?><a x='/>'><b/><c>t</c></a>"),
            2
        );
        assert_eq!(nesting("<a><!-- <b><c> --><![CDATA[<d><e>]]></a>"), 1);
        assert_eq!(nesting(r#"<a><b x=">"><c/></b></a>"#), 3);
        // Unclosed tags (a crafted part) count all the way down.
        let deep = "<a>".repeat(200_000);
        assert_eq!(nesting(&deep), 200_000);
    }

    /// A part nested 200 000 deep is refused (it used to overflow the parser's stack and abort
    /// the process), on a thread with the 2 MiB stack the server's import threads have.
    #[test]
    fn refuses_deeply_nested_xml() {
        std::thread::Builder::new()
            .stack_size(2 << 20)
            .spawn(|| {
                let deep = "<a>".repeat(200_000);
                let e = xml(&deep).expect_err("refused");
                assert!(e.to_string().contains("nested too deeply"), "{e}");
                let ok = format!("{}{}", "<a>".repeat(MAX_DEPTH), "</a>".repeat(MAX_DEPTH));
                assert!(xml(&ok).is_ok());
            })
            .unwrap()
            .join()
            .unwrap();
    }
}
