//! Nexus descriptions (BBCode) to HTML for the mod info dialog's Nexus tab: a port of MO2's
//! `BBCode::convertToHTML` (`modorganizer/src/bbcode.cpp`, 2.5.2; Fluorine's copy is the
//! same), with three of its bugs fixed (on purpose):
//! - MO2's constructor makes every tag pattern case-insensitive and lazy, but `convertTag`
//!   calls `setPatternOptions(DotMatchesEverythingOption)` before each match, which replaces
//!   those options: upper-case tags (`[B]`) stayed as text, and `[email="a@b"]` kept its
//!   closing quote in the address. Here the patterns are case-insensitive and the arguments
//!   lazy, as the author meant.
//! - `[list=1]`'s pattern `\[list.*\](.*)` was greedy, so a numbered list kept only what
//!   followed its last `]`. Here the opening tag ends at its first `]`: every item stays.
//! - A bullet ended only at the next `[*]` or at `</ul>`, so the last bullet of an `[ol]` (or
//!   `[list=1]`) stayed as text. Here it also ends at `</ol>`.
//!
//! The rest is MO2's behaviour:
//! - An unclosed tag gets a closing tag added at the end of the text.
//! - A stray closing tag (`[/b]` with no opening) is dropped; a lone `[/` loses its `[`.
//! - `[color=name]` takes the colour from a small table; an unknown name becomes `#name`.
//! - Only `\r\n` becomes `<br/>`; `\"` and `\'` lose their backslash.

use std::sync::LazyLock;

use regex::{NoExpand, Regex};

/// The tag table: name as found (lower case, with `=` when it takes an argument) to its
/// pattern and replacement (`None`: built by code, the colour tag).
static TAGS: LazyLock<Vec<(&'static str, Regex, Option<&'static str>)>> = LazyLock::new(|| {
    let tag = |name, pattern: &str, replacement| {
        // MO2's constructor meant: dot matches newlines, any case, lazy. The content `(.*)`
        // stays greedy: `convert_tag` cuts the text at the tag's own closing tag first, so
        // greedy and lazy are the same for a single tag, and greedy keeps a nested tag of the
        // same name inside (`[b]a[b]b[/b]c[/b]`). Arguments are lazy where it matters
        // (`[email="..."]`).
        (name, Regex::new(&format!("(?si){pattern}")).expect("a valid tag pattern"), replacement)
    };
    vec![
        tag("b", r"\[b\](.*)\[/b\]", Some("<b>${1}</b>")),
        tag("i", r"\[i\](.*)\[/i\]", Some("<i>${1}</i>")),
        tag("u", r"\[u\](.*)\[/u\]", Some("<u>${1}</u>")),
        tag("s", r"\[s\](.*)\[/s\]", Some("<s>${1}</s>")),
        tag("sub", r"\[sub\](.*)\[/sub\]", Some("<sub>${1}</sub>")),
        tag("sup", r"\[sup\](.*)\[/sup\]", Some("<sup>${1}</sup>")),
        tag("size=", r"\[size=([^\]]*)\](.*)\[/size\]", Some(r#"<font size="${1}">${2}</font>"#)),
        tag("color=", r"\[color=([^\]]*)\](.*)\[/color\]", None),
        tag("font=", r"\[font=([^\]]*)\](.*)\[/font\]", Some(r#"<font style="font-family: ${1};">${2}</font>"#)),
        tag("center", r"\[center\](.*)\[/center\]", Some(r#"<div align="center">${1}</div>"#)),
        tag("right", r"\[right\](.*)\[/right\]", Some(r#"<div align="right">${1}</div>"#)),
        tag("quote", r"\[quote\](.*)\[/quote\]", Some(r#"<figure class="quote"><blockquote>${1}</blockquote></figure>"#)),
        tag(
            "quote=",
            r"\[quote=([^\]]*)\](.*)\[/quote\]",
            Some(r#"<figure class="quote"><blockquote>${2}</blockquote></figure>"#),
        ),
        tag(
            "spoiler",
            r"\[spoiler\](.*)\[/spoiler\]",
            Some(
                "<details><summary>Spoiler:  <div class=\"bbc_spoiler_show\">Show</div></summary><div \
                 class=\"spoiler_content\">${1}</div></details>",
            ),
        ),
        tag("code", r"\[code\](.*)\[/code\]", Some("<code>${1}</code>")),
        tag("heading", r"\[heading\](.*)\[/heading\]", Some("<h2><strong>${1}</strong></h2>")),
        tag("line", r"\[line\]", Some("<hr>")),
        tag("list", r"\[list\](.*)\[/list\]", Some("<ul>${1}</ul>")),
        tag("list=", r"\[list=[^\]]*\](.*)\[/list\]", Some("<ol>${1}</ol>")),
        tag("ul", r"\[ul\](.*)\[/ul\]", Some("<ul>${1}</ul>")),
        tag("ol", r"\[ol\](.*)\[/ol\]", Some("<ol>${1}</ol>")),
        tag("li", r"\[li\](.*)\[/li\]", Some("<li>${1}</li>")),
        tag("table", r"\[table\](.*)\[/table\]", Some("<table>${1}</table>")),
        tag("tr", r"\[tr\](.*)\[/tr\]", Some("<tr>${1}</tr>")),
        tag("th", r"\[th\](.*)\[/th\]", Some("<th>${1}</th>")),
        tag("td", r"\[td\](.*)\[/td\]", Some("<td>${1}</td>")),
        tag("url", r"\[url\](.*)\[/url\]", Some(r#"<a href="${1}">${1}</a>"#)),
        tag("url=", r"\[url=([^\]]*)\](.*)\[/url\]", Some(r#"<a href="${1}">${2}</a>"#)),
        tag("img", r"\[img(?:\s*width=\d+\s*,?\s*height=\d+)?\](.*)\[/img\]", Some(r#"<img src="${1}">"#)),
        tag("img=", r"\[img=([^\]]*)\](.*)\[/img\]", Some(r#"<img src="${2}" alt="${1}">"#)),
        tag("email=", r#"\[email="?([^\]]*?)"?\](.*)\[/email\]"#, Some(r#"<a href="mailto:${1}">${2}</a>"#)),
        tag(
            "youtube",
            r"\[youtube\](.*)\[/youtube\]",
            Some(r#"<a href="https://www.youtube.com/watch?v=${1}">https://www.youtube.com/watch?v=${1}</a>"#),
        ),
        tag("*", r"\[\*\](.*)", Some("<li>${1}</li>")),
    ]
});

/// `m_ColorMap`.
const COLORS: &[(&str, &str)] = &[
    ("red", "FF0000"),
    ("green", "00FF00"),
    ("blue", "0000FF"),
    ("black", "000000"),
    ("gray", "7F7F7F"),
    ("white", "FFFFFF"),
    ("yellow", "FFFF00"),
    ("cyan", "00FFFF"),
    ("magenta", "FF00FF"),
    ("brown", "A52A2A"),
    ("orange", "FFA500"),
    ("gold", "FFD700"),
    ("deepskyblue", "00BFFF"),
    ("salmon", "FA8072"),
    ("dodgerblue", "1E90FF"),
    ("greenyellow", "ADFF2F"),
    ("peru", "CD853F"),
];

/// `nextTag` of every tag that has a closing tag: `[name`, `[name=` or `[name]`, any case.
static NEXT_TAG: LazyLock<Vec<(String, Regex)>> = LazyLock::new(|| {
    TAGS.iter()
        .map(|(key, _, _)| key.trim_end_matches('='))
        .map(|tag| (tag.to_owned(), Regex::new(&format!(r"(?i)\[{}[=\]]?", regex::escape(tag))).expect("a valid pattern")))
        .collect()
});

static TAG_NAME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[a-zA-Z*]*=?").expect("a valid pattern"));
static BULLET_END: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)(\[\*\]|</ul>|</ol>)").expect("a valid pattern"));
static BULLET_TAIL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(\[/\*\])?(<br/>)?$").expect("a valid pattern"));

/// `str::find` from `from` without regard to ASCII case (Qt's `indexOf(.., Qt::CaseInsensitive)`).
fn find_ci(haystack: &str, needle: &str, from: usize) -> Option<usize> {
    let (h, n) = (haystack.as_bytes(), needle.as_bytes());
    (from..=h.len().checked_sub(n.len())?).find(|&i| h[i..i + n.len()].eq_ignore_ascii_case(n))
}

/// `convertTag`: the tag at the start of `input` (which starts with `[`) and what it becomes,
/// with the number of bytes of `input` it took; `None` when it is no tag MO2 knows.
fn convert_tag(input: &str) -> Option<(String, usize)> {
    let name = TAG_NAME.find(&input[1..]).map_or("", |m| m.as_str()).to_lowercase();
    let (_, pattern, replacement) = TAGS.iter().find(|(key, _, _)| *key == name)?;
    let tag = name.strip_suffix('=').unwrap_or(&name);

    let mut input = input.to_owned();
    let (close_pos, close_len) = match tag {
        // a bullet ends at the next bullet or at the end of the list, which it doesn't eat;
        // a bullet outside a list is no tag (as in MO2)
        "*" => (BULLET_END.find_at(&input, 3.min(input.len()))?.start(), 0),
        "line" => (6, 0),
        _ => {
            let next_tag = &NEXT_TAG.iter().find(|(t, _)| t == tag)?.1;
            let close_tag = format!("[/{tag}]");
            let next_from = |input: &str, from: Option<usize>| from.and_then(|f| next_tag.find_at(input, f)).map(|m| m.start());
            let mut close = find_ci(&input, &close_tag, 0);
            let mut next = next_from(&input, input.find(']'));
            // a nested tag of the same name pushes the end out to the next closing tag
            while let (Some(n), Some(c)) = (next, close) {
                if n >= c {
                    break;
                }
                close = find_ci(&input, &close_tag, c + close_tag.len());
                next = next_from(&input, input[n..].find(']').map(|i| i + n));
            }
            let close = match close {
                Some(close) => close,
                None => {
                    // MO2's workaround: an unclosed tag is closed at the end
                    input.push_str(&close_tag);
                    input.len() - close_tag.len()
                }
            };
            (close, close_tag.len())
        }
    };
    let length = (close_pos + close_len).min(input.len());
    let mut temp = input[..length].to_owned();
    let caps = pattern.captures(&temp)?;
    let html = match replacement {
        Some(replacement) => {
            if tag == "*" {
                temp = BULLET_TAIL.replace(&temp, "").into_owned();
            }
            pattern.replace_all(&temp, *replacement).into_owned()
        }
        None => {
            let color = caps.get(1).map_or("", |m| m.as_str());
            let content = caps.get(2).map_or("", |m| m.as_str());
            let font = if color.starts_with('#') {
                format!("<font style=\"color: {color};\">{content}</font>")
            } else {
                let lower = color.to_lowercase();
                let hex = COLORS.iter().find(|(n, _)| *n == lower).map_or(color, |(_, hex)| hex);
                format!("<font style=\"color: #{hex};\">{content}</font>")
            };
            pattern.replace_all(&temp, NoExpand(&font)).into_owned()
        }
    };
    Some((html, length))
}

/// `BBCode::convertToHTML`: one pass over the text, every tag replaced by its HTML, and the
/// result of each tag converted again for the tags nested in it.
pub fn to_html(text: &str) -> String {
    let input = text.replace("\r\n", "<br/>").replace("\\\"", "\"").replace("\\'", "'");
    let mut result = String::with_capacity(input.len());
    let mut last = 0;
    while let Some(found) = input[last..].find('[') {
        let mut pos = last + found;
        result.push_str(&input[last..pos]);
        if input[pos + 1..].starts_with('/') {
            // a closing tag with no opening one is dropped
            pos = match input[pos..].find(']') {
                Some(end) => pos + end + 1,
                None => pos + 1,
            };
        } else {
            match convert_tag(&input[pos..]) {
                Some((html, length)) => {
                    result.push_str(&to_html(&html));
                    pos += length;
                }
                None => {
                    result.push('[');
                    pos += 1;
                }
            }
        }
        // a tag closed by MO2's workaround took more than was left
        last = pos.min(input.len());
    }
    result.push_str(&input[last..]);
    result
}

#[cfg(test)]
mod tests;
