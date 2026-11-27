//! The language pairs offered: OPUS-MT models (Helsinki-NLP, University of Helsinki), each
//! file fetched from its publisher's Hugging Face repository at a pinned commit and checked
//! against a pinned SHA-256.
//!
//! Every pair's licence was read from its model card at the pinned revision (2026-11-26):
//! Apache-2.0 or CC-BY-4.0, both allowing use and redistribution (CC-BY with attribution,
//! which Oriel's Settings ▸ Translation models shows with each pair). Pairs without a public
//! OPUS-MT model (Portuguese → English) are not offered; English → Korean is not offered
//! either: its only model (`opus-mt-tc-big-en-ko`) gives nonsense (tried 2026-11-26; its model
//! card's discussions say the same). Where only the larger "tc-big" model exists (English →
//! Portuguese) it is used.

/// The Hugging Face host the models are fetched from.
pub const HUB: &str = "https://huggingface.co";

/// One file of a model.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelFile {
    /// Its name in the repository (and in the store).
    pub name: String,
    /// Its size, bytes.
    pub size: u64,
    /// Its SHA-256, lowercase hex.
    pub sha256: String,
}

/// A target language a pair's model writes, and the sentence-initial token that asks for it
/// (multi-target models such as English → Chinese need one, e.g. `>>cmn_Hans<<`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    /// BCP 47 code (`es`, `pt-BR`).
    pub language: String,
    /// The token put before the source text, if the model needs one.
    pub token: Option<String>,
}

/// One language pair's model.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pair {
    /// `en-es`: the store's folder name and the API's ID.
    pub id: String,
    /// Source language (BCP 47 primary tag).
    pub from: String,
    /// The target languages it writes (the first is the pair's own).
    pub to: Vec<Target>,
    /// The repository (`Helsinki-NLP/opus-mt-en-es`).
    pub repo: String,
    /// The pinned commit.
    pub revision: String,
    /// SPDX licence of the model, from its model card.
    pub licence: String,
    /// Where files are fetched from (`{base}/{repo}/resolve/{revision}/{file}`).
    pub base_url: String,
    /// Its files: `config.json`, `source.spm`, `target.spm`, `vocab.json` and the weights.
    pub files: Vec<ModelFile>,
}

impl Pair {
    /// The download URL of `file`.
    pub fn url(&self, file: &ModelFile) -> String {
        format!(
            "{}/{}/resolve/{}/{}",
            self.base_url.trim_end_matches('/'),
            self.repo,
            self.revision,
            file.name
        )
    }

    /// The model card's page.
    pub fn card(&self) -> String {
        format!("{HUB}/{}", self.repo)
    }

    /// Total download size, bytes.
    pub fn size(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }

    /// The model's name as shown and recorded (`opus-mt-en-es`).
    pub fn model_name(&self) -> &str {
        self.repo.rsplit('/').next().unwrap_or(&self.repo)
    }

    /// The attribution shown with the pair.
    pub fn attribution(&self) -> String {
        format!(
            "{} by Helsinki-NLP (OPUS-MT, University of Helsinki), {} licence, from {} at revision {}",
            self.model_name(),
            self.licence,
            self.card(),
            &self.revision[..self.revision.len().min(12)]
        )
    }

    /// The target entry for `language` (exact, else by primary tag: `es-MX` → `es`).
    pub fn target(&self, language: &str) -> Option<&Target> {
        let l = language.trim();
        self.to
            .iter()
            .find(|t| t.language.eq_ignore_ascii_case(l))
            .or_else(|| {
                let primary = primary(l);
                self.to
                    .iter()
                    .find(|t| t.language.eq_ignore_ascii_case(primary))
            })
    }
}

/// The primary subtag of a BCP 47 code (`pt-BR` → `pt`).
pub fn primary(language: &str) -> &str {
    language.split(['-', '_']).next().unwrap_or(language).trim()
}

/// The pair translating `from` → `to` in `pairs`.
pub fn find<'a>(pairs: &'a [Pair], from: &str, to: &str) -> Option<&'a Pair> {
    pairs
        .iter()
        .find(|p| p.from.eq_ignore_ascii_case(primary(from)) && p.target(to).is_some())
}

struct P {
    id: &'static str,
    from: &'static str,
    to: &'static [(&'static str, Option<&'static str>)],
    repo: &'static str,
    revision: &'static str,
    licence: &'static str,
    files: &'static [F],
}

struct F(&'static str, u64, &'static str);

const fn f(name: &'static str, size: u64, sha256: &'static str) -> F {
    F(name, size, sha256)
}

/// The pairs offered.
pub fn catalogue() -> Vec<Pair> {
    PAIRS
        .iter()
        .map(|p| Pair {
            id: p.id.into(),
            from: p.from.into(),
            to: p
                .to
                .iter()
                .map(|(l, t)| Target {
                    language: (*l).into(),
                    token: t.map(Into::into),
                })
                .collect(),
            repo: p.repo.into(),
            revision: p.revision.into(),
            licence: p.licence.into(),
            base_url: HUB.into(),
            files: p
                .files
                .iter()
                .map(|F(n, s, h)| ModelFile {
                    name: (*n).into(),
                    size: *s,
                    sha256: (*h).into(),
                })
                .collect(),
        })
        .collect()
}

const PAIRS: &[P] = &[
    P {
        id: "en-es",
        from: "en",
        to: &[("es", None)],
        repo: "Helsinki-NLP/opus-mt-en-es",
        revision: "5bc4493d463cf000c1f0b50f8d56886a392ed4ab",
        licence: "Apache-2.0",
        files: &[
            f(
                "config.json",
                1473,
                "666ad3c2943674b6a789e311d221e44ec23e27f1bd9727930754bf773ec8e464",
            ),
            f(
                "source.spm",
                801636,
                "4dd547c24816a335e7b0b2e63376a8f1b3cbfc671eda5ab808dd44fdadaa8791",
            ),
            f(
                "target.spm",
                825924,
                "e236ee6d866b635c0142114f8647f39831f9d92534aa2aad75c942f6a78ad0e3",
            ),
            f(
                "vocab.json",
                1590040,
                "257f346d7a6b2ecceafcca8ba05648ce2fd68dfaf105fb0e913dca7198f3f6d5",
            ),
            f(
                "pytorch_model.bin",
                312087523,
                "befce0f620e8e63f83bbb7f0818eb51d541cba28057a7cc14b00466d85c4994c",
            ),
        ],
    },
    P {
        id: "es-en",
        from: "es",
        to: &[("en", None)],
        repo: "Helsinki-NLP/opus-mt-es-en",
        revision: "c96e2c5399ebfae4fc43d9669556b9afa74bb69d",
        licence: "Apache-2.0",
        files: &[
            f(
                "config.json",
                1438,
                "374443eacf8986c21386503e2fcd52eae6952ac09c87fd36695f5c12259a4cd1",
            ),
            f(
                "source.spm",
                825924,
                "e236ee6d866b635c0142114f8647f39831f9d92534aa2aad75c942f6a78ad0e3",
            ),
            f(
                "target.spm",
                801636,
                "4dd547c24816a335e7b0b2e63376a8f1b3cbfc671eda5ab808dd44fdadaa8791",
            ),
            f(
                "vocab.json",
                1590040,
                "257f346d7a6b2ecceafcca8ba05648ce2fd68dfaf105fb0e913dca7198f3f6d5",
            ),
            f(
                "pytorch_model.bin",
                312087523,
                "586a4e69e0804459ac7c176a5dc76852652100d9af1ee0db796de719899a5deb",
            ),
        ],
    },
    P {
        id: "en-fr",
        from: "en",
        to: &[("fr", None)],
        repo: "Helsinki-NLP/opus-mt-en-fr",
        revision: "dd7f6540a7a48a7f4db59e5c0b9c42c8eea67f18",
        licence: "Apache-2.0",
        files: &[
            f(
                "config.json",
                1416,
                "df92c371911b45282caac6101fe7e7f5d5e9af7f9a99356067ffe306278ce72b",
            ),
            f(
                "source.spm",
                778395,
                "173e9f493a668fe396d599e28d414a201193094e6ffd7a4678e5aab0f6d3d838",
            ),
            f(
                "target.spm",
                802397,
                "78d0e717c77053f1c4b856d8661d9cb87c64f083a35418c087b9146300e4f585",
            ),
            f(
                "vocab.json",
                1339166,
                "945c604346ce15ce4aff9001001e7f925e336d942c4087017f191871162cbdc4",
            ),
            f(
                "pytorch_model.bin",
                300827685,
                "cc1de10b49342ad2f33e06bc4474ddd6eaca278474903c4a8636ce15680d64de",
            ),
        ],
    },
    P {
        id: "fr-en",
        from: "fr",
        to: &[("en", None)],
        repo: "Helsinki-NLP/opus-mt-fr-en",
        revision: "c4aed37b318c763fd177aa449b44e3b783cc6c02",
        licence: "Apache-2.0",
        files: &[
            f(
                "config.json",
                1416,
                "b3be13d046d9899d7aab8cf4ed624d9a79f5776038ba793f6b4d2ce3e02192f7",
            ),
            f(
                "source.spm",
                802397,
                "78d0e717c77053f1c4b856d8661d9cb87c64f083a35418c087b9146300e4f585",
            ),
            f(
                "target.spm",
                778395,
                "173e9f493a668fe396d599e28d414a201193094e6ffd7a4678e5aab0f6d3d838",
            ),
            f(
                "vocab.json",
                1339166,
                "945c604346ce15ce4aff9001001e7f925e336d942c4087017f191871162cbdc4",
            ),
            f(
                "model.safetensors",
                300803608,
                "6e3837f34b903802c3d0d670362b997cee6e87584a1108eb3fa89e4625e4424a",
            ),
        ],
    },
    P {
        id: "en-de",
        from: "en",
        to: &[("de", None)],
        repo: "Helsinki-NLP/opus-mt-en-de",
        revision: "6183067f769a302e3861815543b9f312c71b0ca4",
        licence: "CC-BY-4.0",
        files: &[
            f(
                "config.json",
                1335,
                "41c7890e6f6c1f72c5c626569cfe739a8362080940edbb1747f25173f541653b",
            ),
            f(
                "source.spm",
                768489,
                "678f2a1177d8389f67b66299762dcc4fc567e89b07e212ba91b0c56daecf47ce",
            ),
            f(
                "target.spm",
                796845,
                "bbd1f495eea99c8e21ae086d9146e0fa7b096c3dfdd9ba07ab8b631889df5c9b",
            ),
            f(
                "vocab.json",
                1273232,
                "0d70d89fee4a8b4ef99a56d712163baadcabd5600a597f71515547ee70306329",
            ),
            f(
                "pytorch_model.bin",
                297928209,
                "da068344b1b0c20e6d4a8f77f48e06646b90d679029006b18579e68977206bdd",
            ),
        ],
    },
    P {
        id: "de-en",
        from: "de",
        to: &[("en", None)],
        repo: "Helsinki-NLP/opus-mt-de-en",
        revision: "1a922f3b32a8e809e17a47d4b32142d8105924e5",
        licence: "Apache-2.0",
        files: &[
            f(
                "config.json",
                1381,
                "89368ef76ea89581025cdf605caac75b8a22af2c1a90ec57c8a5001f10537eeb",
            ),
            f(
                "source.spm",
                796845,
                "bbd1f495eea99c8e21ae086d9146e0fa7b096c3dfdd9ba07ab8b631889df5c9b",
            ),
            f(
                "target.spm",
                768489,
                "678f2a1177d8389f67b66299762dcc4fc567e89b07e212ba91b0c56daecf47ce",
            ),
            f(
                "vocab.json",
                1273232,
                "0d70d89fee4a8b4ef99a56d712163baadcabd5600a597f71515547ee70306329",
            ),
            f(
                "pytorch_model.bin",
                297928209,
                "e743c3070f61f477cb62fe95ef2c9be2e77f3e488cb6b8030ff8a19e8295c87d",
            ),
        ],
    },
    P {
        id: "en-pt",
        from: "en",
        to: &[("pt", Some(">>por<<")), ("pt-BR", Some(">>pob<<"))],
        repo: "Helsinki-NLP/opus-mt-tc-big-en-pt",
        revision: "9f2863d807ecf91a374bdbecb8d01e402e90622e",
        licence: "CC-BY-4.0",
        files: &[
            f(
                "config.json",
                1076,
                "ca76b1818f066007e94fb2519c0752320cee36a5d4947bf7ef4477c845feacc5",
            ),
            f(
                "source.spm",
                802741,
                "7a7fcf812cf03a5785daa35d4932bbbe69e7e605c0fe56fce5a3f731d6c355aa",
            ),
            f(
                "target.spm",
                824855,
                "b3cf20ccd4e48a89882cfdba157c05fa1cd71a6b69517dfc80e3a34d387ea91f",
            ),
            f(
                "vocab.json",
                1380764,
                "dad10ad0acbf34ad92af16cb37fd71732d2b73851274698d58c5439386b506a1",
            ),
            f(
                "model.safetensors",
                465035080,
                "f1772ec97f6cb5b942bb6a5555a04272960a228a523f7ed47e24014236aa1716",
            ),
        ],
    },
    P {
        id: "ko-en",
        from: "ko",
        to: &[("en", None)],
        repo: "Helsinki-NLP/opus-mt-ko-en",
        revision: "e42d1f41b66194e6d10512f8a27bebc1f4f5097e",
        licence: "Apache-2.0",
        files: &[
            f(
                "config.json",
                1394,
                "abe3c36f13da71bf49f175b114d8415dce5301330f849f66a1aeccd5d177a581",
            ),
            f(
                "source.spm",
                841805,
                "c9496f7c2be9aecb84c751ae9f35a875915dde8e3892f652a5c76811ab2a0f49",
            ),
            f(
                "target.spm",
                813126,
                "f1531ac8576fe64267dfca0dc33950a6e9a2d3fd9e05346558ad7ea5ee0e65bf",
            ),
            f(
                "vocab.json",
                1719866,
                "d42e3838e7ed0e43785fa013ef22ab4d382248cdf4e959c46a2b48db5eb019e9",
            ),
            f(
                "pytorch_model.bin",
                312087009,
                "4b2209fbd0c58a05e9c4b818b49b6b7e54406cdee93acdccf840752a94fe34f4",
            ),
        ],
    },
    P {
        id: "en-zh",
        from: "en",
        to: &[
            ("zh", Some(">>cmn_Hans<<")),
            ("zh-Hans", Some(">>cmn_Hans<<")),
            ("zh-Hant", Some(">>cmn_Hant<<")),
        ],
        repo: "Helsinki-NLP/opus-mt-en-zh",
        revision: "408d9bc410a388e1d9aef112a2daba955b945255",
        licence: "Apache-2.0",
        files: &[
            f(
                "config.json",
                1403,
                "bcae8ed74fed77fb51c58462b62397fee6b1a1a34aece79183a0dd02ad329e71",
            ),
            f(
                "source.spm",
                806435,
                "5775ddc9e3ff2fae91554da56468ad35ff56edaba870fea74447bc7234bfdaa8",
            ),
            f(
                "target.spm",
                804600,
                "81dc94efa84e4025ef38d25d5d07429fe41e3eb29d44003f1db6fe98487b0052",
            ),
            f(
                "vocab.json",
                1617791,
                "f1832ca38f7aa158b9a944ff583e4901e6b37d9d4ad9241f03838e636e9cfb03",
            ),
            f(
                "pytorch_model.bin",
                312087009,
                "69a1d6ec829cee349360b3b677ac0aa99a7d88822d1a6578370029efffdca3f5",
            ),
        ],
    },
    P {
        id: "zh-en",
        from: "zh",
        to: &[("en", None)],
        repo: "Helsinki-NLP/opus-mt-zh-en",
        revision: "cf109095479db38d6df799875e34039d4938aaa6",
        licence: "CC-BY-4.0",
        files: &[
            f(
                "config.json",
                1394,
                "d84104da12fa90da59acf5f0ed30428e43cc92574ad4b9dfa88f29eeeb2b0eb0",
            ),
            f(
                "source.spm",
                804677,
                "e27a3a1b539f4959ec72ea60e453f49156289f95d4e6000b29332efc45616203",
            ),
            f(
                "target.spm",
                806530,
                "6a881f4717cd7265f53fea54fd3dc689c767c05338fac7a4590f3088cb2d7855",
            ),
            f(
                "vocab.json",
                1617902,
                "c0f79ee4c413ccb24cd808a0c727eec85b2130f23c82802cc1caeb07b5f0d458",
            ),
            f(
                "pytorch_model.bin",
                312087009,
                "9d8ceb91d103ef89400c9d9d62328b4858743cf8924878aee3b8afc594242ce0",
            ),
        ],
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_pair_is_pinned_and_complete() {
        let pairs = catalogue();
        assert!(pairs.len() >= 7);
        for p in &pairs {
            assert_eq!(p.revision.len(), 40, "{}: a full commit hash", p.id);
            assert!(matches!(p.licence.as_str(), "Apache-2.0" | "CC-BY-4.0"));
            let names: Vec<_> = p.files.iter().map(|f| f.name.as_str()).collect();
            for need in ["config.json", "source.spm", "target.spm", "vocab.json"] {
                assert!(names.contains(&need), "{}: {need}", p.id);
            }
            assert!(
                names.contains(&"model.safetensors") || names.contains(&"pytorch_model.bin"),
                "{}: weights",
                p.id
            );
            for f in &p.files {
                assert_eq!(f.sha256.len(), 64);
                assert!(f.sha256.bytes().all(|b| b.is_ascii_hexdigit()));
            }
        }
        for (from, to) in [
            ("en", "es"),
            ("es", "en"),
            ("en", "fr"),
            ("en", "de"),
            ("en", "pt"),
            ("en", "pt-BR"),
            ("en", "zh"),
        ] {
            assert!(find(&pairs, from, to).is_some(), "{from} → {to}");
        }
        assert_eq!(
            find(&pairs, "en-US", "es-MX").map(|p| p.id.as_str()),
            Some("en-es")
        );
        assert!(find(&pairs, "es", "fr").is_none());
        // English → Korean is not offered: its only OPUS-MT model translates wrongly.
        assert!(find(&pairs, "en", "ko").is_none());
        let zh = find(&pairs, "en", "zh-Hant").unwrap();
        assert_eq!(
            zh.target("zh-Hant").unwrap().token.as_deref(),
            Some(">>cmn_Hant<<")
        );
    }
}
