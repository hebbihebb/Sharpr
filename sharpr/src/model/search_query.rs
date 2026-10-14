//! Filter words typed in the search box (decision D16 in
//! `docs/reference/product.md`).
//!
//! Recognized words become chips: file formats, resolutions ("4k" means at
//! least 3840×2160 in either orientation, "below 4k" the opposite) and
//! aspects. Everything else stays search text. A word in double quotes is
//! always text, which is how a chip is turned back into a search word.
//! Chips of the same kind are alternatives (`png webp`); different kinds
//! must all match (`png 4k`). A leading `-` negates: `-png` and `-16:9`
//! leave out a format or aspect, and any other `-word` leaves out images
//! like that word (file names containing it, and, with visual search,
//! the images closest to it).

use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Jpeg,
    Png,
    Webp,
    Jxl,
    Gif,
    Tiff,
    Bmp,
}

impl Format {
    fn parse(word: &str) -> Option<Self> {
        Some(match word {
            "jpg" | "jpeg" => Self::Jpeg,
            "png" => Self::Png,
            "webp" => Self::Webp,
            "jxl" => Self::Jxl,
            "gif" => Self::Gif,
            "tif" | "tiff" => Self::Tiff,
            "bmp" => Self::Bmp,
            _ => return None,
        })
    }

    fn extensions(self) -> &'static [&'static str] {
        match self {
            Self::Jpeg => &["jpg", "jpeg"],
            Self::Png => &["png"],
            Self::Webp => &["webp"],
            Self::Jxl => &["jxl"],
            Self::Gif => &["gif"],
            Self::Tiff => &["tif", "tiff"],
            Self::Bmp => &["bmp"],
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Jpeg => "JPEG",
            Self::Png => "PNG",
            Self::Webp => "WebP",
            Self::Jxl => "JPEG XL",
            Self::Gif => "GIF",
            Self::Tiff => "TIFF",
            Self::Bmp => "BMP",
        }
    }
}

/// A resolution class, as long edge × short edge so it fits either orientation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Resolution {
    pub word: &'static str,
    pub long: u32,
    pub short: u32,
}

const RESOLUTIONS: [Resolution; 6] = [
    Resolution {
        word: "720p",
        long: 1280,
        short: 720,
    },
    Resolution {
        word: "1080p",
        long: 1920,
        short: 1080,
    },
    Resolution {
        word: "1440p",
        long: 2560,
        short: 1440,
    },
    Resolution {
        word: "4k",
        long: 3840,
        short: 2160,
    },
    Resolution {
        word: "5k",
        long: 5120,
        short: 2880,
    },
    Resolution {
        word: "8k",
        long: 7680,
        short: 4320,
    },
];

impl Resolution {
    fn parse(word: &str) -> Option<Self> {
        RESOLUTIONS.into_iter().find(|r| r.word == word)
    }

    fn reached_by(self, (w, h): (u32, u32)) -> bool {
        w.max(h) >= self.long && w.min(h) >= self.short
    }

    fn label(self) -> String {
        self.word.to_uppercase().replace('P', "p")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Aspect {
    Landscape,
    Portrait,
    Square,
    /// Width:height within 2%, matching the existing 16:9 filter's tolerance.
    Ratio(u32, u32),
}

impl Aspect {
    fn parse(word: &str) -> Option<Self> {
        Some(match word {
            "landscape" => Self::Landscape,
            "portrait" => Self::Portrait,
            "square" | "1:1" => Self::Square,
            "16:9" => Self::Ratio(16, 9),
            "21:9" => Self::Ratio(21, 9),
            "16:10" => Self::Ratio(16, 10),
            "4:3" => Self::Ratio(4, 3),
            "3:2" => Self::Ratio(3, 2),
            _ => return None,
        })
    }

    fn matches(self, (w, h): (u32, u32)) -> bool {
        match self {
            Self::Landscape => w > h,
            Self::Portrait => h > w,
            Self::Square => w == h,
            Self::Ratio(a, b) => {
                ((w as f64 / h as f64) / (a as f64 / b as f64) - 1.0).abs() <= 0.020_000_000_1
            }
        }
    }

    fn label(self) -> String {
        match self {
            Self::Landscape => "Landscape".into(),
            Self::Portrait => "Portrait".into(),
            Self::Square => "Square".into(),
            Self::Ratio(a, b) => format!("{a}:{b}"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Chip {
    Format(Format),
    AtLeast(Resolution),
    Below(Resolution),
    Aspect(Aspect),
    /// A negated filter (`-png`, `-16:9`): every one must hold.
    Not(Box<Chip>),
    /// A negative word (`-water`), lowercase.
    Without(String),
}

impl Chip {
    /// Text shown on the chip.
    pub fn label(&self) -> String {
        match self {
            Self::Format(format) => format.label().into(),
            Self::AtLeast(resolution) => format!("{}+", resolution.label()),
            Self::Below(resolution) => format!("Below {}", resolution.label()),
            Self::Aspect(aspect) => aspect.label(),
            Self::Not(inner) => format!("Not {}", inner.label()),
            Self::Without(word) => format!("Not {word}"),
        }
    }

    /// A negative word, if this chip is one.
    pub fn without(&self) -> Option<&str> {
        match self {
            Self::Without(word) => Some(word),
            _ => None,
        }
    }

    /// The text that turns this chip back into a plain search word.
    pub fn as_literal(&self) -> String {
        // A negation comes back as "-word", left for the user to edit.
        match self {
            Self::Not(inner) => return format!("-{}", inner.as_literal().trim_matches('"')),
            Self::Without(word) => return format!("-{word}"),
            _ => {}
        }
        let word = match self {
            Self::Format(format) => format.extensions()[0].to_string(),
            Self::AtLeast(resolution) | Self::Below(resolution) => resolution.word.to_string(),
            Self::Aspect(aspect) => match aspect {
                Aspect::Landscape => "landscape".into(),
                Aspect::Portrait => "portrait".into(),
                Aspect::Square => "square".into(),
                Aspect::Ratio(a, b) => format!("{a}:{b}"),
            },
            Self::Not(_) | Self::Without(_) => unreachable!("returned above"),
        };
        format!("\"{word}\"")
    }

    /// Positive chips of one kind are alternatives; `None` for negations,
    /// which must all hold.
    fn kind(&self) -> Option<u8> {
        match self {
            Self::Format(_) => Some(0),
            Self::AtLeast(_) | Self::Below(_) => Some(1),
            Self::Aspect(_) => Some(2),
            Self::Not(_) | Self::Without(_) => None,
        }
    }

    fn needs_dimensions(&self) -> bool {
        match self {
            Self::Format(_) | Self::Without(_) => false,
            Self::Not(inner) => inner.needs_dimensions(),
            _ => true,
        }
    }

    fn matches(&self, path: &Path, dimensions: Option<(u32, u32)>) -> bool {
        match self {
            Self::Format(format) => path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|ext| format.extensions().contains(&ext.to_lowercase().as_str())),
            Self::AtLeast(resolution) => dimensions.is_some_and(|d| resolution.reached_by(d)),
            Self::Below(resolution) => dimensions.is_some_and(|d| !resolution.reached_by(d)),
            Self::Aspect(aspect) => dimensions
                .filter(|(w, h)| *w > 0 && *h > 0)
                .is_some_and(|d| aspect.matches(d)),
            Self::Not(inner) => !inner.matches(path, dimensions),
            // The visual part of a negative word is applied by the search.
            Self::Without(word) => !path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|name| name.to_lowercase().contains(word.as_str())),
        }
    }
}

/// Typed chips applied together: alternatives within a kind, all kinds required.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ChipFilters {
    pub chips: Vec<Chip>,
}

impl ChipFilters {
    pub fn is_empty(&self) -> bool {
        self.chips.is_empty()
    }

    pub fn needs_dimensions(&self) -> bool {
        self.chips.iter().any(Chip::needs_dimensions)
    }

    pub fn matches(&self, path: &Path, dimensions: Option<(u32, u32)>) -> bool {
        let positives = (0..3).all(|kind| {
            let mut of_kind = self
                .chips
                .iter()
                .filter(|c| c.kind() == Some(kind))
                .peekable();
            of_kind.peek().is_none() || of_kind.any(|chip| chip.matches(path, dimensions))
        });
        positives
            && self
                .chips
                .iter()
                .filter(|c| c.kind().is_none())
                .all(|chip| chip.matches(path, dimensions))
    }

    /// The negative words, whose visual part the search applies.
    pub fn without_words(&self) -> Vec<String> {
        self.chips
            .iter()
            .filter_map(|c| c.without().map(str::to_string))
            .collect()
    }
}

/// Search text split into chips and the remaining words.
#[derive(Debug, Default, PartialEq)]
pub struct ParsedQuery {
    pub chips: Vec<Chip>,
    pub text: String,
}

fn parse_words(words: &[&str]) -> ParsedQuery {
    let mut parsed = ParsedQuery::default();
    let mut text = Vec::new();
    let mut i = 0;
    while i < words.len() {
        let word = words[i];
        let lower = word.to_lowercase();
        let below = matches!(lower.as_str(), "below" | "under")
            .then(|| words.get(i + 1).map(|w| w.to_lowercase()))
            .flatten()
            .and_then(|next| Resolution::parse(&next));
        let chip = if let Some(resolution) = below {
            i += 1;
            Some(Chip::Below(resolution))
        } else if let Some(rest) = lower.strip_prefix('<') {
            Resolution::parse(rest).map(Chip::Below)
        } else if let Some(rest) = lower.strip_prefix('-').filter(|r| !r.is_empty()) {
            Some(
                Format::parse(rest)
                    .map(Chip::Format)
                    .or_else(|| Resolution::parse(rest).map(Chip::AtLeast))
                    .or_else(|| Aspect::parse(rest).map(Chip::Aspect))
                    .map(|chip| Chip::Not(Box::new(chip)))
                    .unwrap_or_else(|| Chip::Without(rest.to_string())),
            )
        } else {
            Format::parse(&lower)
                .map(Chip::Format)
                .or_else(|| Resolution::parse(&lower).map(Chip::AtLeast))
                .or_else(|| Aspect::parse(&lower).map(Chip::Aspect))
        };
        match chip {
            Some(chip) => {
                if !parsed.chips.contains(&chip) {
                    parsed.chips.push(chip);
                }
            }
            None => text.push(
                word.strip_prefix('"')
                    .and_then(|w| w.strip_suffix('"'))
                    .filter(|w| !w.is_empty())
                    .unwrap_or(word),
            ),
        }
        i += 1;
    }
    parsed.text = text.join(" ");
    parsed
}

/// Parses every word, including one still being typed: used for searching.
pub fn parse(input: &str) -> ParsedQuery {
    let words: Vec<&str> = input.split_whitespace().collect();
    parse_words(&words)
}

/// Takes chips out of the words the user has finished (followed by a space)
/// and returns them with the text that should remain in the search box.
/// A recognized word still being typed stays in the box. Quoted words and
/// other text keep their original spelling.
pub fn take_completed_chips(input: &str) -> (Vec<Chip>, String) {
    let ends_with_space = input.ends_with(char::is_whitespace);
    let words: Vec<&str> = input.split_whitespace().collect();
    let (done, typing) = if ends_with_space || words.is_empty() {
        (&words[..], None)
    } else {
        (&words[..words.len() - 1], words.last().copied())
    };
    let mut chips = Vec::new();
    let mut kept: Vec<&str> = Vec::new();
    let mut i = 0;
    while i < done.len() {
        let lower = done[i].to_lowercase();
        let pair_below = matches!(lower.as_str(), "below" | "under")
            && done
                .get(i + 1)
                .is_some_and(|next| Resolution::parse(&next.to_lowercase()).is_some());
        let span = if pair_below {
            &done[i..i + 2]
        } else {
            &done[i..i + 1]
        };
        let parsed = parse_words(span);
        if parsed.text.is_empty() && !parsed.chips.is_empty() {
            for chip in parsed.chips {
                if !chips.contains(&chip) {
                    chips.push(chip);
                }
            }
        } else {
            kept.extend_from_slice(span);
        }
        i += span.len();
    }
    // "below" just typed, waiting for its resolution: keep it in the box.
    let mut remaining = kept.join(" ");
    if let Some(typing) = typing {
        if !remaining.is_empty() {
            remaining.push(' ');
        }
        remaining.push_str(typing);
    } else if !remaining.is_empty() {
        remaining.push(' ');
    }
    (chips, remaining)
}

/// Filter words offered while typing, in display order.
const SUGGESTED_WORDS: &[&str] = &[
    "4k",
    "1080p",
    "1440p",
    "5k",
    "8k",
    "720p",
    "below 4k",
    "below 1440p",
    "below 1080p",
    "16:9",
    "21:9",
    "16:10",
    "4:3",
    "3:2",
    "landscape",
    "portrait",
    "square",
    "png",
    "jpg",
    "webp",
    "jxl",
    "tiff",
    "gif",
    "bmp",
];

/// Filter words that complete the word being typed (the last word, when the
/// input does not end in a space), most useful first.
pub fn filter_suggestions(input: &str) -> Vec<&'static str> {
    if input.ends_with(char::is_whitespace) {
        return Vec::new();
    }
    let words: Vec<String> = input.split_whitespace().map(str::to_lowercase).collect();
    let Some(partial) = words.last() else {
        return Vec::new();
    };
    // "below" or "under" followed by the start of a resolution.
    let after_below =
        words.len() >= 2 && matches!(words[words.len() - 2].as_str(), "below" | "under");
    SUGGESTED_WORDS
        .iter()
        .copied()
        .filter(|word| {
            if after_below {
                word.strip_prefix("below ")
                    .is_some_and(|r| r.starts_with(partial.as_str()))
            } else {
                word.starts_with(partial.as_str()) && *word != partial.as_str()
            }
        })
        .take(6)
        .collect()
}

/// Replaces the word being typed with `suggestion` and a trailing space, so
/// the next search turns it into a chip.
pub fn apply_suggestion(input: &str, suggestion: &str) -> String {
    let mut words: Vec<&str> = input.split_whitespace().collect();
    words.pop();
    if suggestion.starts_with("below ")
        && words
            .last()
            .is_some_and(|w| matches!(w.to_lowercase().as_str(), "below" | "under"))
    {
        words.pop();
    }
    words.push(suggestion);
    format!("{} ", words.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn four_k() -> Resolution {
        Resolution::parse("4k").unwrap()
    }

    #[test]
    fn filter_words_become_chips_and_the_rest_stays_text() {
        let parsed = parse("Mountains 4K PNG 16:9 lake");
        assert_eq!(parsed.text, "Mountains lake");
        assert_eq!(
            parsed.chips,
            vec![
                Chip::AtLeast(four_k()),
                Chip::Format(Format::Png),
                Chip::Aspect(Aspect::Ratio(16, 9)),
            ]
        );
    }

    #[test]
    fn below_and_quoted_words() {
        assert_eq!(parse("below 4k").chips, vec![Chip::Below(four_k())]);
        assert_eq!(parse("<4k sunset").chips, vec![Chip::Below(four_k())]);
        let quoted = parse("town \"square\"");
        assert!(quoted.chips.is_empty());
        assert_eq!(quoted.text, "town square");
        assert_eq!(parse("below the waves").text, "below the waves");
    }

    #[test]
    fn only_completed_words_leave_the_search_box() {
        let (chips, rest) = take_completed_chips("mountains 4k");
        assert!(chips.is_empty());
        assert_eq!(rest, "mountains 4k");

        let (chips, rest) = take_completed_chips("mountains 4k ");
        assert_eq!(chips, vec![Chip::AtLeast(four_k())]);
        assert_eq!(rest, "mountains ");

        let (chips, rest) = take_completed_chips("png below 4k sea");
        assert_eq!(
            chips,
            vec![Chip::Format(Format::Png), Chip::Below(four_k())]
        );
        assert_eq!(rest, "sea");

        let (chips, rest) = take_completed_chips("\"square\" park ");
        assert!(chips.is_empty());
        assert_eq!(rest, "\"square\" park ");
    }

    #[test]
    fn chips_match_alternatives_within_a_kind_and_all_kinds() {
        let filters = ChipFilters {
            chips: parse("png webp 4k landscape").chips,
        };
        let png = Path::new("/a/sea.PNG");
        let jpg = Path::new("/a/sea.jpg");
        assert!(filters.matches(png, Some((3840, 2160))));
        assert!(filters.matches(Path::new("/a/sea.webp"), Some((5120, 2880))));
        assert!(!filters.matches(jpg, Some((3840, 2160))), "format");
        assert!(!filters.matches(png, Some((2560, 1440))), "resolution");
        assert!(!filters.matches(png, Some((2160, 3840))), "orientation");
        assert!(!filters.matches(png, None), "unknown size");
    }

    #[test]
    fn resolution_counts_either_orientation_and_below_is_its_opposite() {
        let at_least = ChipFilters {
            chips: vec![Chip::AtLeast(four_k())],
        };
        let below = ChipFilters {
            chips: vec![Chip::Below(four_k())],
        };
        let p = Path::new("/a/x.jpg");
        assert!(at_least.matches(p, Some((2160, 3840))));
        assert!(!below.matches(p, Some((2160, 3840))));
        assert!(below.matches(p, Some((1920, 1080))));
        assert!(!below.matches(p, None));
    }

    #[test]
    fn suggestions_complete_the_word_being_typed() {
        assert_eq!(filter_suggestions("mountains 4"), vec!["4k", "4:3"]);
        assert_eq!(
            filter_suggestions("be"),
            vec!["below 4k", "below 1440p", "below 1080p"]
        );
        assert_eq!(
            filter_suggestions("sea below 1"),
            vec!["below 1440p", "below 1080p"]
        );
        assert!(
            filter_suggestions("mountains 4k ").is_empty(),
            "a finished word is done"
        );
        assert!(filter_suggestions("zebra").is_empty());
        assert!(filter_suggestions("").is_empty());

        assert_eq!(apply_suggestion("mountains 4", "4k"), "mountains 4k ");
        assert_eq!(
            apply_suggestion("sea below 1", "below 1080p"),
            "sea below 1080p "
        );
        let (chips, rest) = take_completed_chips(&apply_suggestion("sea be", "below 4k"));
        assert_eq!(chips, vec![Chip::Below(four_k())]);
        assert_eq!(rest, "sea ");
    }

    #[test]
    fn chips_turn_back_into_text_that_stays_text() {
        let chip = Chip::Aspect(Aspect::Square);
        assert_eq!(chip.label(), "Square");
        let literal = chip.as_literal();
        assert!(parse(&literal).chips.is_empty());
        assert_eq!(parse(&literal).text, "square");
        assert_eq!(Chip::AtLeast(four_k()).label(), "4K+");
        assert_eq!(
            Chip::Below(Resolution::parse("1080p").unwrap()).label(),
            "Below 1080p"
        );
    }

    #[test]
    fn a_leading_minus_negates_filters_and_words() {
        let parsed = parse("tower -water -png -16:9 -4k");
        assert_eq!(parsed.text, "tower");
        assert_eq!(
            parsed.chips,
            vec![
                Chip::Without("water".into()),
                Chip::Not(Box::new(Chip::Format(Format::Png))),
                Chip::Not(Box::new(Chip::Aspect(Aspect::Ratio(16, 9)))),
                Chip::Not(Box::new(Chip::AtLeast(four_k()))),
            ]
        );
        let filters = ChipFilters {
            chips: parsed.chips.clone(),
        };
        assert_eq!(filters.without_words(), vec!["water".to_string()]);
        let wide = Some((1920, 1080));
        let boxy = Some((1600, 1200));
        assert!(filters.matches(Path::new("/l/tower.jpg"), boxy));
        assert!(!filters.matches(Path::new("/l/tower.png"), boxy), "-png");
        assert!(!filters.matches(Path::new("/l/tower.jpg"), wide), "-16:9");
        assert!(
            !filters.matches(Path::new("/l/Waterfall.jpg"), boxy),
            "name has water"
        );
        assert!(
            !filters.matches(Path::new("/l/big.jpg"), Some((4000, 3000))),
            "-4k"
        );
        assert_eq!(Chip::Without("water".into()).label(), "Not water");
        assert_eq!(
            Chip::Not(Box::new(Chip::Format(Format::Png))).label(),
            "Not PNG"
        );
        assert_eq!(Chip::Without("water".into()).as_literal(), "-water");
        assert_eq!(
            Chip::Not(Box::new(Chip::Format(Format::Png))).as_literal(),
            "-png"
        );
        // Negations are all required, alongside positive alternatives.
        let mixed = ChipFilters {
            chips: parse("png webp -16:9").chips,
        };
        assert!(mixed.matches(Path::new("/l/a.webp"), boxy));
        assert!(!mixed.matches(Path::new("/l/a.webp"), wide));
        assert!(parse("\"-water\"").chips.is_empty(), "quoted stays text");
        assert!(parse("-").chips.is_empty());
    }

    #[test]
    fn finished_negative_words_become_chips() {
        let (chips, remaining) = take_completed_chips("tower -water ");
        assert_eq!(chips, vec![Chip::Without("water".into())]);
        assert_eq!(remaining, "tower ");
    }
}
