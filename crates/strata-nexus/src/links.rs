//! `nxm://` and `modl://` links as pure functions: uibase's `NXMUrl` (nxm and old-style modl
//! links of the same shape) and the Leet Edition's `modl://` handling (Leet change 8:
//! `isModlLink`, `modlDownloadUrl`, `modSiteName`, `modSiteFromDownloadUrl`, `modPubModId`,
//! `modPubPageUrl` in `shared/util.cpp`).

/// What a link asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Link {
    /// `nxm://<game>/mods/<mod>/files/<file>?key=..&expires=..&user_id=..` (also with
    /// `modl://`): a Nexus file.
    NexusFile { game: String, mod_id: u64, file_id: u64, key: Option<String>, expires: Option<i64>, user_id: Option<i64> },
    /// `nxm://<game>/collections/<slug>/revisions/<n>`.
    NexusCollection { game: String, slug: String, revision: u64 },
    /// `modl://<game>/?url=<percent-encoded https link>`: a download from another site
    /// (ModPub, DwemerMods); `site` as `modSiteFromDownloadUrl` names it.
    SiteDownload { game: String, url: String, site: String },
}

/// Why a link was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LinkError {
    #[error("\"{0}\" is not an nxm:// or modl:// link")]
    NotALink(String),
    #[error("\"{0}\" is not a link Strata understands (InvalidNXMLinkException)")]
    Invalid(String),
    #[error("the modl:// link \"{0}\" carries no https download address")]
    NoDownload(String),
}

/// `%XX` decoding (`+` stays a plus, as `QUrlQuery` leaves it in `FullyDecoded`).
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Some(v) = std::str::from_utf8(&bytes[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The value of `key` in a query string, decoded.
fn query_value(query: &str, key: &str) -> Option<String> {
    query.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        (percent_decode(k) == key).then(|| percent_decode(v))
    })
}

/// `isModlLink`.
pub fn is_modl_link(link: &str) -> bool {
    link.len() >= 7 && link[..7].eq_ignore_ascii_case("modl://")
}

/// Reads a link.
pub fn parse(link: &str) -> Result<Link, LinkError> {
    let link = link.trim();
    let (scheme, rest) = link.split_once("://").ok_or_else(|| LinkError::NotALink(link.to_owned()))?;
    let scheme = scheme.to_ascii_lowercase();
    if scheme != "nxm" && scheme != "modl" {
        return Err(LinkError::NotALink(link.to_owned()));
    }
    let (before_query, query) = rest.split_once('?').unwrap_or((rest, ""));
    let (host, path) = before_query.split_once('/').unwrap_or((before_query, ""));
    let game = host.to_ascii_lowercase();
    let game_ok = !game.is_empty() && game.bytes().all(|b| b.is_ascii_alphanumeric());
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    let number = |s: &str| s.parse::<u64>().ok().filter(|_| s.bytes().all(|b| b.is_ascii_digit()));

    if game_ok {
        match parts.as_slice() {
            [m, mod_id, f, file_id, ..] if m.eq_ignore_ascii_case("mods") && f.eq_ignore_ascii_case("files") => {
                if let (Some(mod_id), Some(file_id)) = (number(mod_id), number(file_id)) {
                    return Ok(Link::NexusFile {
                        game,
                        mod_id,
                        file_id,
                        key: query_value(query, "key"),
                        expires: query_value(query, "expires").and_then(|v| v.parse().ok()),
                        user_id: query_value(query, "user_id").and_then(|v| v.parse().ok()),
                    });
                }
            }
            [c, slug, r, revision, ..] if c.eq_ignore_ascii_case("collections") && r.eq_ignore_ascii_case("revisions") => {
                if let Some(revision) = number(revision)
                    && slug.bytes().all(|b| b.is_ascii_alphanumeric())
                {
                    return Ok(Link::NexusCollection { game, slug: slug.to_ascii_lowercase(), revision });
                }
            }
            _ => {}
        }
    }
    if scheme == "modl" {
        // modlDownloadUrl: only an https address is taken
        return match query_value(query, "url") {
            Some(url) if url.len() >= 8 && url[..8].eq_ignore_ascii_case("https://") => {
                let site = mod_site_from_download_url(&url);
                Ok(Link::SiteDownload { game, url, site })
            }
            _ => Err(LinkError::NoDownload(link.to_owned())),
        };
    }
    Err(LinkError::Invalid(link.to_owned()))
}

/// `modSiteName`: the site's name for a host, subdomains included; else the host itself.
pub fn mod_site_name(host: &str) -> String {
    const NAMES: [(&str, &str); 3] = [("dwemermods.com", "DwemerMods"), ("dwemer-mods.com", "DwemerMods"), ("mod.pub", "ModPub")];
    let h = host.to_lowercase();
    NAMES
        .iter()
        .find(|(domain, _)| h == *domain || h.ends_with(&format!(".{domain}")))
        .map_or_else(|| host.to_owned(), |(_, name)| (*name).to_owned())
}

/// The host of an http(s) address.
fn host_of(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let authority = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    authority.split(':').next().unwrap_or("")
}

/// `modSiteFromDownloadUrl`: ModPub files come from cloud storage but are always named
/// `...[mod.pub].<ext>`; otherwise the host decides.
pub fn mod_site_from_download_url(url: &str) -> String {
    if percent_decode(url).to_lowercase().contains("[mod.pub]") {
        return "ModPub".into();
    }
    mod_site_name(host_of(url))
}

/// `<name>-<mod id>-<version>-<yyyy.mm.dd>-[mod.pub].<ext>` → (name, mod id).
fn match_modpub_file_name(file_name: &str) -> Option<(String, u64)> {
    let name = file_name.rsplit(['/', '\\']).next()?;
    let lower = name.to_lowercase();
    let at = lower.rfind("-[mod.pub].")?;
    let ext = &name[at + "-[mod.pub].".len()..];
    if ext.is_empty() || ext.contains('.') {
        return None;
    }
    let head = &name[..at]; // <name>-<id>-<version>-<date>
    let (rest, date) = head.rsplit_once('-')?;
    let d = date.as_bytes();
    let date_ok = d.len() == 10
        && d.iter().enumerate().all(|(i, c)| if i == 4 || i == 7 { *c == b'.' } else { c.is_ascii_digit() });
    if !date_ok {
        return None;
    }
    let (rest, version) = rest.rsplit_once('-')?;
    if version.contains('-') {
        return None;
    }
    let (title, id) = rest.rsplit_once('-')?;
    let id = id.parse::<u64>().ok().filter(|_| id.bytes().all(|b| b.is_ascii_digit()))?;
    Some((title.to_owned(), id))
}

/// `modPubModId`.
pub fn modpub_mod_id(file_name: &str) -> Option<u64> {
    match_modpub_file_name(file_name).map(|(_, id)| id)
}

/// `modPubPageUrl`: `https://mod.pub/<game>/<id>-<slug>`, the slug guessed from the file
/// name (the site redirects a wrong slug). Only Skyrim SE's page name is known.
pub fn modpub_page_url(file_name: &str, game_short_name: &str) -> Option<String> {
    let game = match game_short_name.to_lowercase().as_str() {
        "skyrimse" => "skyrim-se",
        _ => return None,
    };
    let (title, id) = match_modpub_file_name(file_name)?;
    let mut slug = String::new();
    for c in title.to_lowercase().chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            slug.push(c);
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug = slug.trim_matches('-');
    let slug = if slug.is_empty() { "mod" } else { slug };
    Some(format!("https://mod.pub/{game}/{id}-{slug}"))
}

/// The name of Strata's link handler entry in `~/.local/share/applications`.
pub const DESKTOP_FILE: &str = "strata-links.desktop";

/// The desktop entry that makes `exec` (Strata's binary) the handler of `nxm://` and
/// `modl://` links: `strata link <url>`. The path is quoted as the Desktop Entry
/// specification asks (`"`, `` ` ``, `$` and `\` escaped inside quotes).
pub fn desktop_entry(exec: &str) -> String {
    // plain when nothing needs quoting: xdg-mime takes the Exec line's first word as the
    // program, quotes included, and drops a handler whose program it then cannot find
    let plain = exec.chars().all(|c| c.is_ascii_alphanumeric() || "/._-+".contains(c));
    let mut quoted = String::from(if plain { "" } else { "\"" });
    for c in exec.chars() {
        if matches!(c, '"' | '`' | '$' | '\\') {
            quoted.push('\\');
        }
        quoted.push(c);
    }
    if !plain {
        quoted.push('"');
    }
    format!(
        "[Desktop Entry]\nType=Application\nName=Strata link handler\nComment=Opens nxm:// and modl:// links in Strata\n\
         Exec={quoted} link %u\nMimeType=x-scheme-handler/nxm;x-scheme-handler/modl;\nNoDisplay=true\nTerminal=false\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nexus_links_parse_as_nxmurl() {
        assert_eq!(
            parse("nxm://SkyrimSpecialEdition/mods/12604/files/35407?key=abc%2Fd&expires=1700000000&user_id=42").unwrap(),
            Link::NexusFile {
                game: "skyrimspecialedition".into(),
                mod_id: 12604,
                file_id: 35407,
                key: Some("abc/d".into()),
                expires: Some(1_700_000_000),
                user_id: Some(42),
            }
        );
        assert_eq!(
            parse("nxm://skyrimspecialedition/collections/abc123/revisions/7").unwrap(),
            Link::NexusCollection { game: "skyrimspecialedition".into(), slug: "abc123".into(), revision: 7 }
        );
        assert!(matches!(parse("nxm://skyrimse/mods/x/files/1"), Err(LinkError::Invalid(_))));
        assert!(matches!(parse("https://nexusmods.com"), Err(LinkError::NotALink(_))));
        // modl links of the nxm shape are Nexus files too (NXMUrl's regex takes both)
        assert!(matches!(parse("modl://skyrimse/mods/1/files/2").unwrap(), Link::NexusFile { mod_id: 1, file_id: 2, .. }));
    }

    #[test]
    fn modl_links_carry_a_download() {
        let link = "modl://skyrimse/?url=https%3A%2F%2Fcdn.dwemermods.com%2Ffiles%2FCool%20Mod.7z";
        assert!(is_modl_link(link) && is_modl_link("MODL://x"));
        assert_eq!(
            parse(link).unwrap(),
            Link::SiteDownload {
                game: "skyrimse".into(),
                url: "https://cdn.dwemermods.com/files/Cool Mod.7z".into(),
                site: "DwemerMods".into(),
            }
        );
        let modpub = "modl://skyrimse/?url=https%3A%2F%2Fpub-1.r2.dev%2FBetter%20Ores-123-1.0-2026.10.13-%5Bmod.pub%5D.zip";
        assert!(matches!(parse(modpub).unwrap(), Link::SiteDownload { site, .. } if site == "ModPub"));
        assert!(matches!(parse("modl://skyrimse/?url=http%3A%2F%2Fx.com%2Fa.7z"), Err(LinkError::NoDownload(_))));
        assert_eq!(mod_site_name("example.org"), "example.org");
        assert_eq!(mod_site_name("MOD.PUB"), "ModPub");
    }

    #[test]
    fn the_desktop_entry_quotes_the_binary() {
        let entry = desktop_entry("/home/user/My $Apps/strata");
        assert!(entry.contains("Exec=\"/home/user/My \\$Apps/strata\" link %u\n"), "{entry}");
        assert!(entry.contains("MimeType=x-scheme-handler/nxm;x-scheme-handler/modl;\n"));
        // nothing to quote: plain, as xdg-mime needs it
        assert!(desktop_entry("/home/user/.local/share/strata/bin/strata").contains("Exec=/home/user/.local/share/strata/bin/strata link %u\n"));
    }

    #[test]
    fn modpub_file_names() {
        let name = "Better Ores & Gems-1234-1.0.2-2026.10.13-[mod.pub].7z";
        assert_eq!(modpub_mod_id(name), Some(1234));
        assert_eq!(modpub_page_url(name, "SkyrimSE").as_deref(), Some("https://mod.pub/skyrim-se/1234-better-ores-gems"));
        assert_eq!(modpub_page_url(name, "Oblivion"), None);
        assert_eq!(modpub_mod_id("Some-Mod-1.0.7z"), None);
        assert_eq!(modpub_mod_id("x-12-v1-2026.1.13-[mod.pub].7z"), None);
    }
}
