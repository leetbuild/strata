//! Web pages the window opens.

/// The game's page on Nexus Mods, as MO2's "Visit Nexus" opens it
/// (`NexusInterface::getGameURL`: `https://www.nexusmods.com/` + the game plugin's
/// `gameNexusName`, lower case). `game_name` is `ModOrganizer.ini`'s `gameName`; the Nexus
/// names are the game plugins' (Fluorine `libs/game_bethesda`: `gameskyrimse.cpp`,
/// `gameskyrim.cpp`, `gameoblivion.cpp`). `None` for a game Strata doesn't know.
pub fn nexus_game_url(game_name: &str) -> Option<String> {
    let nexus = match game_name {
        "Skyrim Special Edition" => "skyrimspecialedition",
        "Skyrim" => "skyrim",
        "Oblivion" => "oblivion",
        _ => return None,
    };
    Some(format!("https://www.nexusmods.com/{nexus}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn game_pages_by_the_name_in_the_ini() {
        assert_eq!(nexus_game_url("Skyrim Special Edition").as_deref(), Some("https://www.nexusmods.com/skyrimspecialedition"));
        assert_eq!(nexus_game_url("Oblivion").as_deref(), Some("https://www.nexusmods.com/oblivion"));
        assert_eq!(nexus_game_url("Starfield"), None);
    }
}
