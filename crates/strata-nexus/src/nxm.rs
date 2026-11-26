//! An `nxm://` link to a queued download, as MO2's `DownloadManager::addNXMDownload` →
//! `nxmFileInfoAvailable` → `nxmDownloadURLsAvailable` → `addDownload` do it: the file's
//! info, the mod's name and category (`nxmDescriptionAvailable`), then the download links
//! (with the link's `key` and `expires`, which free accounts need).

use crate::api::Client;
use crate::queue::DownloadInfo;
use crate::{Error, Result};

/// What the queue needs for one Nexus file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prepared {
    /// The servers' URLs, first choice first (MO2 sorts by the preferred servers; Strata keeps
    /// Nexus' order).
    pub urls: Vec<String>,
    /// The archive's name in the downloads folder (`file_name` of the file info).
    pub file_name: String,
    pub info: DownloadInfo,
}

/// `IPluginGame::gameShortName` for a Nexus game domain (`validShortNames` of the game
/// plugins Strata has).
pub fn game_short_name(domain: &str) -> Option<&'static str> {
    match domain.to_ascii_lowercase().as_str() {
        "skyrimspecialedition" => Some("SkyrimSE"),
        "skyrim" => Some("Skyrim"),
        "oblivion" => Some("Oblivion"),
        _ => None,
    }
}

/// Asks Nexus about the file of a link and its download links.
pub fn prepare(client: &Client, game: &str, mod_id: i64, file_id: i64, nxm: Option<(&str, i64)>) -> Result<Prepared> {
    let file = client.file_info(game, mod_id, file_id)?;
    let module = client.mod_info(game, mod_id)?;
    let links = client.download_links(game, mod_id, file_id, nxm)?;
    if links.is_empty() {
        return Err(Error::Api { status: 0, message: "No download server available. Please try again later.".to_owned() });
    }
    // info->version.parse(...); invalid (empty) -> newestVersion (the mod's version)
    let version = if file.version.is_empty() { module.version.clone() } else { file.version.clone() };
    let file_name = if file.file_name.is_empty() { format!("{}-{mod_id}-{file_id}", file.name) } else { file.file_name.clone() };
    if file_name.contains('/') || file_name.contains('\\') || file_name.starts_with('.') {
        return Err(Error::Format(format!("unusable file name from Nexus: {file_name}")));
    }
    Ok(Prepared {
        urls: links.into_iter().map(|l| l.uri).collect(),
        file_name,
        info: DownloadInfo {
            game_name: game_short_name(game).unwrap_or(game).to_owned(),
            mod_id,
            file_id,
            name: file.name,
            description: file.description,
            mod_name: module.name,
            version,
            newest_version: module.version,
            file_category: file.category_id,
            category: module.category_id,
            repository: "Nexus".to_owned(),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{Auth, Http};
    use std::sync::atomic::AtomicBool;

    #[test]
    fn link_to_finished_download_against_a_fake_nexus() {
        let server = crate::fake::serve();
        let data: Vec<u8> = (0..300_000u32).map(|i| (i * 7) as u8).collect();
        server.add(crate::fake::nexus_file_routes(
            &server.base,
            "skyrimspecialedition",
            5,
            77,
            "Cool Mod",
            "Cool Mod-5-1-2.7z",
            "1.2",
            data.clone(),
        ));
        let http = Http::default();
        let client = Client::new(&http, Auth::ApiKey("K".into())).with_base(&server.api());
        let p = prepare(&client, "skyrimspecialedition", 5, 77, Some(("abc", 1_800_000_000))).unwrap();
        assert_eq!(p.file_name, "Cool Mod-5-1-2.7z");
        assert_eq!((p.info.game_name.as_str(), p.info.mod_name.as_str(), p.info.version.as_str()), ("SkyrimSE", "Cool Mod", "1.2"));
        assert_eq!((p.info.category, p.info.file_category), (42, 1));
        let requests = server.requests.lock().unwrap().clone();
        assert!(
            requests.iter().any(|r| r
                == "GET /v1/games/skyrimspecialedition/mods/5/files/77/download_link?key=abc&expires=1800000000 apikey=K"),
            "{requests:?}"
        );

        let dir = tempfile::tempdir().unwrap();
        let mut queue = crate::queue::Queue::new(dir.path().to_owned());
        queue.push(&p.urls[0], &p.file_name, p.info.clone());
        let (_, done) = queue.run_next(&mut |_, _, _| {}, &AtomicBool::new(false)).unwrap();
        let path = match done.unwrap() {
            crate::queue::Finished::Done(path) => path,
            other => panic!("{other:?}"),
        };
        assert_eq!(std::fs::read(&path).unwrap(), data);
        let meta = std::fs::read_to_string(dir.path().join("Cool Mod-5-1-2.7z.meta")).unwrap();
        for line in ["modID=5", "fileID=77", "modName=Cool Mod", "gameName=SkyrimSE", "version=1.2", "installed=false"] {
            assert!(meta.contains(line), "{line} in {meta}");
        }
    }

    #[test]
    fn no_server_and_odd_names_are_refused() {
        let server = crate::fake::serve();
        let mut routes = crate::fake::nexus_file_routes(&server.base, "skyrim", 1, 2, "M", "../evil.7z", "1", vec![1]);
        server.add(routes.clone());
        let http = Http::default();
        let client = Client::new(&http, Auth::ApiKey("K".into())).with_base(&server.api());
        assert!(prepare(&client, "skyrim", 1, 2, None).is_err(), "a path in the file name");
        routes.insert("/v1/games/skyrim/mods/1/files/2/download_link".into(), b"[]".to_vec());
        server.add(routes);
        let err = prepare(&client, "skyrim", 1, 2, None).unwrap_err().to_string();
        assert!(err.contains("No download server"), "{err}");
    }
}
