use super::to_html;

/// Real descriptions from a real instance's `mods/*/meta.ini` (`nexusDescription`, as
/// `ModMeta` reads it), with the HTML MO2's conversion gives for them.
#[test]
fn real_descriptions() {
    // Courier Notification
    assert_eq!(
        to_html("are new to read.\n<br />[color=#ffff00]A note[/color] (undelivered)\n<br />The esp has only one form"),
        "are new to read.\n<br /><font style=\"color: #ffff00;\">A note</font> (undelivered)\n<br />The esp has only one form"
    );
    // Address Library for SKSE Plugins
    assert_eq!(
        to_html(
            "[color=#ff7700][size=4][b]Description[/b][/size][/color]\n<br />\n<br />[b]For regular mod users:[/b] \
             Download and install the \"all-in-one\" package"
        ),
        "<font style=\"color: #ff7700;\"><font size=\"4\"><b>Description</b></font></font>\n<br />\n<br />\
         <b>For regular mod users:</b> Download and install the \"all-in-one\" package"
    );
    // Beard Mask Fix
    assert_eq!(
        to_html(
            "[line]\n<br />[center][url=https://ko-fi.com/parapets][img]https://cdn.ko-fi.com/cdn/kofi2.png[/img][/url][/center]"
        ),
        "<hr>\n<br /><div align=\"center\"><a href=\"https://ko-fi.com/parapets\">\
         <img src=\"https://cdn.ko-fi.com/cdn/kofi2.png\"></a></div>"
    );
    // Enchantable Special Item Fix: a list, each bullet ends at the next or at the list's end
    assert_eq!(
        to_html(
            "Patches provided:\n<br />[list]\n<br />[*][b]Creation Club Fearsome Fists[/b] - Adds a bonus.\n<br />\
             [*][b]Unofficial Skyrim Special Edition Patch[/b] - Restores descriptions.\n<br />[/list]"
        ),
        "Patches provided:\n<br /><ul>\n<br /><li><b>Creation Club Fearsome Fists</b> - Adds a bonus.\n<br /></li>\
         <li><b>Unofficial Skyrim Special Edition Patch</b> - Restores descriptions.\n<br /></li></ul>"
    );
}

#[test]
fn the_other_tags() {
    assert_eq!(to_html("[i]a[/i][u]b[/u][s]c[/s][sub]d[/sub][sup]e[/sup]"), "<i>a</i><u>b</u><s>c</s><sub>d</sub><sup>e</sup>");
    assert_eq!(to_html("[font=Arial]x[/font]"), "<font style=\"font-family: Arial;\">x</font>");
    assert_eq!(to_html("[right]x[/right]"), "<div align=\"right\">x</div>");
    assert_eq!(to_html("[quote=Bob]x[/quote]"), "<figure class=\"quote\"><blockquote>x</blockquote></figure>");
    assert_eq!(to_html("[code]x[/code][heading]h[/heading]"), "<code>x</code><h2><strong>h</strong></h2>");
    assert_eq!(to_html("[list][*]a[*]b[/list]"), "<ul><li>a</li><li>b</li></ul>");
    assert_eq!(to_html("[table][tr][td]1[/td][/tr][/table]"), "<table><tr><td>1</td></tr></table>");
    assert_eq!(to_html("[url]http://x[/url]"), "<a href=\"http://x\">http://x</a>");
    assert_eq!(to_html("[img width=10, height=20]p.png[/img]"), "<img src=\"p.png\">");
    assert_eq!(to_html("[email=a@b.c]mail[/email]"), "<a href=\"mailto:a@b.c\">mail</a>");
    assert_eq!(
        to_html("[youtube]abc[/youtube]"),
        "<a href=\"https://www.youtube.com/watch?v=abc\">https://www.youtube.com/watch?v=abc</a>"
    );
    assert!(to_html("[spoiler]s[/spoiler]").contains("<div class=\"spoiler_content\">s</div></details>"));
    // nested tags of one name: the outer one ends at its own closing tag
    assert_eq!(to_html("[b]a[b]b[/b]c[/b]"), "<b>a<b>b</b>c</b>");
}

/// MO2's behaviour that is kept (see the module doc).
#[test]
fn mo2s_rules() {
    assert_eq!(to_html("[color=red]r[/color]"), "<font style=\"color: #FF0000;\">r</font>");
    assert_eq!(to_html("[color=teal]t[/color]"), "<font style=\"color: #teal;\">t</font>");
    assert_eq!(to_html("[b]unclosed"), "<b>unclosed</b>");
    assert_eq!(to_html("stray[/b] end"), "stray end");
    assert_eq!(to_html("[left]unknown[/left]"), "[left]unknown");
    assert_eq!(to_html("a\r\nb \\\"q\\\""), "a<br/>b \"q\"");
    assert_eq!(to_html("[ not a tag ["), "[ not a tag [");
}

/// The three MO2 bugs fixed on purpose (on purpose). MO2 gave
/// `[B]upper`, `<ol>b</ol>` and `<ol><li>a</li>[*]b</ol>` for the first three.
#[test]
fn mo2s_bugs_are_fixed() {
    // tags in any case
    assert_eq!(to_html("[B]upper[/B]"), "<b>upper</b>");
    assert_eq!(to_html("[Color=Red]r[/COLOR] [URL=http://x]x[/Url]"), "<font style=\"color: #FF0000;\">r</font> <a href=\"http://x\">x</a>");
    // a numbered list keeps every item
    assert_eq!(to_html("[list=1][*]a[*]b[/list]"), "<ol><li>a</li><li>b</li></ol>");
    // the last bullet of an [ol] is a bullet too
    assert_eq!(to_html("[ol][*]a[*]b[/ol]"), "<ol><li>a</li><li>b</li></ol>");
    // a bullet outside a list stays text, as in MO2
    assert_eq!(to_html("[spoiler][*]x[/spoiler]"), to_html("[spoiler]PLACEHOLDER[/spoiler]").replace("PLACEHOLDER", "[*]x"));
    // a lazy argument: the closing quote is not part of the address
    assert_eq!(to_html("[email=\"a@b.c\"]mail[/email]"), "<a href=\"mailto:a@b.c\">mail</a>");
    // nesting still works
    assert_eq!(to_html("[B]a[b]b[/B]c[/b]"), "<b>a<b>b</b>c</b>");
}

/// Every description of a real instance's mods (the first of `STRATA_REAL_INSTANCES`, read
/// only): no panic, and no tag Strata knows is left as text, in any case.
#[test]
fn every_real_description_converts() {
    let instances = std::env::var("STRATA_REAL_INSTANCES").unwrap_or_default();
    let Some(first) = instances.split(':').find(|s| !s.is_empty()) else {
        return eprintln!("SKIPPED: every_real_description_converts: STRATA_REAL_INSTANCES is not set (real data, read only)");
    };
    let mods = std::path::Path::new(first).join("mods");
    let Ok(entries) = std::fs::read_dir(&mods) else {
        eprintln!("SKIPPED: every_real_description_converts: no {} (real data, read only)", mods.display());
        return;
    };
    let mut converted = 0;
    for entry in entries.flatten() {
        let Ok(meta) = strata_core::ModMeta::open(&entry.path()) else { continue };
        let text = meta.nexus_description();
        if text.is_empty() {
            continue;
        }
        let html = to_html(&text);
        let lower = html.to_lowercase();
        for tag in ["[b]", "[i]", "[u]", "[center]", "[spoiler]", "[line]", "[list", "[url", "[color", "[size"] {
            assert!(!lower.contains(tag), "{tag} left in {}", entry.path().display());
        }
        converted += 1;
    }
    eprintln!("{converted} descriptions converted");
}
