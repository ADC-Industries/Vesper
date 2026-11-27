//! Themes: named collections of template slides.
//!
//! A theme is a set of full-slide layouts ("Lyrics", "Scripture", "Banner Lyrics"), not a
//! style sheet. Template elements carry [`Placeholder`](crate::element::Placeholder) slots and
//! the `themed` flag. Applying a template slide **copies** it into the content slide
//! ([`crate::template::apply_template`]), like ProPresenter, and the slide keeps
//! `templateOrigin` provenance. A slide can also **follow** its template slide (a linked
//! theme, [`crate::theme_link`]): the server then keeps it resolved as the theme changes.
//!
//! Theme media (backgrounds, picture frames) are [`MediaAsset`](crate::media::MediaAsset)s with
//! `scope: theme` and an [`AssetLocation::ThemeAsset`](crate::media::AssetLocation) location,
//! so a theme exports and deletes as a unit. Byte-identical duplicate themes are de-duplicated
//! by the importer.

use crate::common::{ImportInfo, Link, Size, SortKey, is_default};
use crate::ids::{GuideId, TemplateSlideId, ThemeId};
use crate::slide::{Slide, TemplateRef};
use serde::{Deserialize, Serialize};

/// A theme (stored document, including its template slides).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct Theme {
    /// Schema version.
    #[serde(default = "crate::migrate::schema_v1")]
    pub schema_version: u32,
    /// Theme ID.
    pub id: ThemeId,
    /// Name ("Banner Styles"), from the folder name on import.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// Folder path for grouping in the browser (`["Styles"]`), outermost first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub folder: Vec<String>,
    /// Order among themes.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub sort_key: SortKey,
    /// Canvas size of every template slide, px.
    #[serde(default, skip_serializing_if = "is_default")]
    pub canvas_size: Size,
    /// Template slides in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub slides: Vec<TemplateSlide>,
    /// How second-language text looks on slides styled from this theme (see
    /// [`crate::language`]).
    #[serde(default, skip_serializing_if = "crate::common::is_default")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub second_language: crate::language::SecondLanguageStyle,
    /// Import provenance (legacy internal name of macOS-authored themes in `originalName`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub import: Option<ImportInfo>,
}

impl Default for Theme {
    fn default() -> Self {
        Theme {
            schema_version: crate::SCHEMA_VERSION,
            id: ThemeId::new(),
            name: String::new(),
            folder: Vec::new(),
            sort_key: SortKey::new(),
            canvas_size: Size::HD,
            slides: Vec::new(),
            second_language: Default::default(),
            import: None,
        }
    }
}

impl Theme {
    /// Template slide by ID.
    pub fn template(&self, id: TemplateSlideId) -> Option<&TemplateSlide> {
        self.slides.iter().find(|t| t.id == id)
    }

    /// A reference to one of this theme's template slides (with the theme name as hint).
    pub fn template_ref(&self, id: TemplateSlideId) -> TemplateRef {
        TemplateRef {
            theme: Link::named(self.id, self.name.clone()),
            slide: id,
        }
    }

    /// Applies template slide `id` to `slide` (copy semantics, see
    /// [`crate::template::apply_template`]) and records `templateOrigin`.
    /// `None` when the theme has no such template slide.
    pub fn apply(&self, id: TemplateSlideId, slide: &Slide) -> Option<Slide> {
        let t = self.template(id)?;
        let mut out = crate::template::apply_template(slide, &t.slide);
        out.template_origin = Some(self.template_ref(id));
        out = crate::theme_link::unlink(&out);
        Some(out)
    }
}

/// A named template slide.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct TemplateSlide {
    /// Template slide ID (referenced by looks, `templateOrigin` and messages).
    pub id: TemplateSlideId,
    /// Layout name ("Lyrics", "Scripture"). Names are reused across themes by convention.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// The layout. Its actions (e.g. a background video) are copied onto slides that get the
    /// template; its size is the theme's `canvasSize`.
    pub slide: Slide,
    /// Editor alignment guides.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub guides: Vec<Guide>,
    /// Import provenance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub import: Option<ImportInfo>,
}

/// An editor alignment guide line.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct Guide {
    /// Guide ID.
    pub id: GuideId,
    /// Orientation.
    #[serde(default, skip_serializing_if = "is_default")]
    pub orientation: GuideOrientation,
    /// Position in px: x for vertical guides, y for horizontal ones.
    #[serde(default, skip_serializing_if = "crate::common::is_zero")]
    pub position: f64,
}

/// Guide orientation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum GuideOrientation {
    /// A vertical line.
    #[default]
    Vertical,
    /// A horizontal line.
    Horizontal,
    /// Unknown value.
    #[serde(other)]
    Unknown,
}
