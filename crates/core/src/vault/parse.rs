//! Per-note text rules: tags, wikilinks and the plain-text excerpt.
//!
//! These mirror `extractTags`, `extractWikilinks` and `noteExcerpt` in the web version's
//! `parser.js` exactly, including JavaScript's definitions of whitespace and string
//! length, so the parity test can require identical output.

use std::sync::LazyLock;

use regex::Regex;

/// JavaScript's `\s`: WhiteSpace plus LineTerminator. Rust's `\s` differs (it includes
/// U+0085 and excludes U+FEFF), so every pattern that means `\s` uses this instead.
const JS_WS: &str = r"\t\n\x0B\x0C\r \x{A0}\x{1680}\x{2000}-\x{200A}\x{2028}\x{2029}\x{202F}\x{205F}\x{3000}\x{FEFF}";

fn re(pattern: &str) -> Regex {
    Regex::new(&pattern.replace("{WS}", JS_WS)).expect("valid regex")
}

static FRONTMATTER: LazyLock<Regex> = LazyLock::new(|| re(r"\A---\r?\n((?s:.*?))\r?\n---"));
static TAGS_FLOW: LazyLock<Regex> = LazyLock::new(|| re(r"(?m)^tags:[{WS}]*\[([^\]]*)\]"));
static TAGS_BLOCK: LazyLock<Regex> = LazyLock::new(|| {
    re(r"(?m)^tags:[ \t]*\r?\n((?:[ \t]*-[ \t]+[^{WS}][^\r\n]*(?:\r?\n|$))+)")
});
// JS `.` stops at \r, \n, U+2028 and U+2029.
static BLOCK_ITEM: LazyLock<Regex> = LazyLock::new(|| re(r"\A[{WS}]*-[{WS}]+([^\r\n\x{2028}\x{2029}]+)"));
static TAGS_INLINE: LazyLock<Regex> = LazyLock::new(|| re(r"(?:\A|[{WS}])#([a-zA-Z][A-Za-z0-9_\-/]*)"));
static WIKILINK: LazyLock<Regex> = LazyLock::new(|| re(r"(!?)\[\[([^\[\]|#]+)(?:[|#][^\[\]]*)?\]\]"));
static NON_NOTE_EXT: LazyLock<Regex> = LazyLock::new(|| {
    re(r"(?i)\.(png|jpe?g|gif|webp|svg|bmp|avif|heic|pdf|mp3|mp4|m4a|wav|ogg|webm|mov|canvas|excalidraw|base|csv|zip)$")
});
static ANY_EXT: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)\.[a-z0-9]{1,6}$"));
static MD_EXT: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)\.md$"));

/// JavaScript's `String.prototype.trim`.
pub(crate) fn js_trim(s: &str) -> &str {
    s.trim_matches(is_js_ws)
}

fn is_js_ws(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\u{0B}' | '\u{0C}' | '\r' | ' ' | '\u{A0}' | '\u{1680}' | '\u{2000}'..='\u{200A}'
        | '\u{2028}' | '\u{2029}' | '\u{202F}' | '\u{205F}' | '\u{3000}' | '\u{FEFF}')
}

/// Tags in order of first appearance: frontmatter flow list, frontmatter block list,
/// then inline `#tags` in the body. Deduplicated.
pub fn extract_tags(content: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut add = |t: &str| {
        let v: String = js_trim(t).chars().filter(|c| !matches!(c, '\'' | '"' | '#')).collect();
        if !v.is_empty() && !out.contains(&v) {
            out.push(v);
        }
    };
    let fm = FRONTMATTER.captures(content);
    if let Some(fm) = &fm {
        let inner = fm.get(1).map_or("", |m| m.as_str());
        if let Some(flow) = TAGS_FLOW.captures(inner) {
            flow[1].split(',').for_each(&mut add);
        }
        if let Some(block) = TAGS_BLOCK.captures(inner) {
            for line in split_lines(&block[1]) {
                if let Some(m) = BLOCK_ITEM.captures(line) {
                    add(&m[1]);
                }
            }
        }
    }
    let body = match &fm {
        Some(fm) => &content[fm.get(0).unwrap().end()..],
        None => content,
    };
    for m in TAGS_INLINE.captures_iter(body) {
        add(&m[1]);
    }
    out
}

/// `str.split(/\r?\n/)`.
fn split_lines(s: &str) -> impl Iterator<Item = &str> {
    s.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l))
}

/// Wikilink targets that point at notes, in order, `.md` suffix removed. Links to
/// attachments (images, PDFs, audio...) and embeds of non-markdown files are skipped.
pub fn extract_wikilinks(content: &str) -> Vec<String> {
    let mut out = Vec::new();
    for m in WIKILINK.captures_iter(content) {
        let target = js_trim(&m[2]);
        if target.is_empty() || NON_NOTE_EXT.is_match(target) {
            continue;
        }
        if &m[1] == "!" && ANY_EXT.is_match(target) && !MD_EXT.is_match(target) {
            continue;
        }
        out.push(MD_EXT.replace(target, "").into_owned());
    }
    out
}

static EX_FENCE: LazyLock<Regex> = LazyLock::new(|| re(r"(?s)```.*?```"));
static EX_EMBED: LazyLock<Regex> = LazyLock::new(|| re(r"!\[\[[^\]]*\]\]"));
static EX_WIKI: LazyLock<Regex> = LazyLock::new(|| re(r"\[\[([^\]|#]+)(?:[|#]([^\]]*))?\]\]"));
static EX_MDLINK: LazyLock<Regex> = LazyLock::new(|| re(r"\[([^\]]*)\]\([^)]*\)"));
static EX_SYMBOLS: LazyLock<Regex> = LazyLock::new(|| re(r"[#>*_`~|\-]+"));
static EX_SPACES: LazyLock<Regex> = LazyLock::new(|| re(r"[{WS}]+"));
static EX_TAIL_WORD: LazyLock<Regex> = LazyLock::new(|| re(r"[{WS}]+[^{WS}]*$"));

/// Plain-text excerpt for the "note of the hour" overlay: no frontmatter or code blocks,
/// links collapsed to their text, markdown symbols and whitespace squashed. Lengths are
/// counted in UTF-16 units, as JavaScript does.
pub fn note_excerpt(content: &str, max_length: usize) -> String {
    let text = FRONTMATTER.replace(content, "");
    let text = EX_FENCE.replace_all(&text, " ");
    let text = EX_EMBED.replace_all(&text, " ");
    let text = EX_WIKI.replace_all(&text, |c: &regex::Captures| {
        match c.get(2).map(|m| m.as_str()) {
            Some(alias) if !alias.is_empty() => alias.to_string(),
            _ => c[1].to_string(),
        }
    });
    let text = EX_MDLINK.replace_all(&text, "$1");
    let text = EX_SYMBOLS.replace_all(&text, " ");
    let text = EX_SPACES.replace_all(&text, " ");
    let text = js_trim(&text);
    if utf16_len(text) <= max_length {
        return text.to_string();
    }
    let mut units = 0;
    let cut = text
        .char_indices()
        .find(|(_, c)| {
            units += c.len_utf16();
            units > max_length.saturating_sub(1)
        })
        .map_or(text.len(), |(i, _)| i);
    let head = EX_TAIL_WORD.replace(&text[..cut], "");
    format!("{head}…")
}

pub(crate) fn utf16_len(s: &str) -> usize {
    s.chars().map(char::len_utf16).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_from_all_three_sources_in_order() {
        let note = "---\ntags: [alpha, \"beta\"]\n---\nbody #gamma and #alpha #nested/tag";
        assert_eq!(extract_tags(note), ["alpha", "beta", "gamma", "nested/tag"]);
        let block = "---\ntitle: x\ntags:\n  - one\n  - 'two'\n---\n#three";
        assert_eq!(extract_tags(block), ["one", "two", "three"]);
    }

    #[test]
    fn inline_tags_need_whitespace_before_hash() {
        assert_eq!(extract_tags("#a#b c#d #9x #e"), ["a", "e"]);
    }

    #[test]
    fn crlf_frontmatter() {
        assert_eq!(extract_tags("---\r\ntags:\r\n  - x\r\n---\r\n#y"), ["x", "y"]);
    }

    #[test]
    fn wikilinks_skip_attachments_and_strip_md() {
        let note = "[[A]] [[B|alias]] [[C#Heading]] ![[pic.png]] [[doc.pdf]] ![[D.md]] ![[E.mov]] [[F.md]] ![[G]] [[ ]]";
        assert_eq!(extract_wikilinks(note), ["A", "B", "C", "D", "F", "G"]);
    }

    #[test]
    fn excerpt_collapses_links_and_truncates() {
        let note = "---\na: 1\n---\n# Title\nSee [[Target|the alias]] and [[Other]] and [md](http://x).\n```\ncode\n```\nEnd";
        assert_eq!(note_excerpt(note, 220), "Title See the alias and Other and md. End");
        let long = "word ".repeat(100);
        let ex = note_excerpt(&long, 20);
        assert!(ex.ends_with('…') && utf16_len(&ex) <= 20, "{ex}");
    }
}
