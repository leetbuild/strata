//! Text put into the HTML of tool tips and dialogs (Qt's rich text).

/// `&`, `<` and `>` escaped, as `QString::toHtmlEscaped` does for these (quotes are left:
/// the texts go into element content, never into attributes).
pub fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    #[test]
    fn markup_characters_are_escaped_ampersand_first() {
        assert_eq!(super::escape("a <b> & c&lt;"), "a &lt;b&gt; &amp; c&amp;lt;");
    }
}
