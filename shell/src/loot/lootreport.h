#pragma once

// MO2's Loot::Report and its parts (loot.h / loot.cpp): the report a sort returns (JSON from
// crates/strata-ffi/src/bridge_loot.rs, lootcli's names), the markdown the LOOT dialog shows,
// and what the plugin list adds from it (PluginList::addLootReport, makeLootTooltip,
// iconData).

#include <QJsonObject>
#include <QString>
#include <QStringList>

namespace loot
{

// Loot::Report::toMarkdown. `okay` false: LOOT failed (`errors` says why). `report` is the
// JSON of a sort that worked (empty otherwise).
QString reportMarkdown(bool okay, const QStringList& errors, const QStringList& warnings,
                       const QJsonObject& report);

// One plugin's part (Loot::Plugin::toMarkdown): "" when it has nothing to say.
QString pluginMarkdown(const QJsonObject& plugin);

// Loot::Dirty::toString.
QString dirtyString(const QJsonObject& dirty, bool isClean);

// PluginList::makeLootTooltip: "" when there is nothing.
QString pluginTooltip(const QJsonObject& plugin);

// PluginList::iconData with a plugin's LOOT info: the warning icon (incompatibilities, missing
// masters or requirements) first, the information icon (messages) after the lock, the
// cleaning icon (dirty) last.
QStringList pluginIcons(QStringList icons, const QJsonObject& plugin);

}  // namespace loot
