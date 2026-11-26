#include "lootreport.h"

#include <QCoreApplication>
#include <QJsonArray>

namespace loot
{

namespace
{

QString tr(const char* text)
{
  return QCoreApplication::translate("Loot", text);
}

// Loot::Message::toMarkdown
QString messageMarkdown(const QJsonObject& m)
{
  QString s;
  const QString type = m.value("type").toString();
  if (type == "error") {
    s += "**" + tr("Error") + "**: ";
  } else if (type == "warn") {
    s += "**" + tr("Warning") + "**: ";
  }
  return s + m.value("text").toString();
}

QString fileName(const QJsonValue& f)
{
  const QJsonObject o = f.toObject();
  const QString display = o.value("displayName").toString();
  return display.isEmpty() ? o.value("name").toString() : display;
}

QString joined(const QJsonArray& files)
{
  QStringList names;
  for (const auto& f : files) {
    names << (f.isString() ? f.toString() : fileName(f));
  }
  return names.join(", ");
}

// Loot::Stats::toMarkdown; Strata in lootcli's place, libloot in LOOT's.
QString statsMarkdown(const QJsonObject& stats)
{
  return QString("`stats: %1s, strata %2, libloot %3`")
      .arg(QString::number(stats.value("time").toDouble() / 1000.0, 'f', 2))
      .arg(stats.value("strataVersion").toString())
      .arg(stats.value("lootVersion").toString());
}

// What Strata adds before the messages: whether the order changed, a masterlist that could not
// be downloaded.
QString outcomeMarkdown(const QJsonObject& report)
{
  QString s;
  const int moved = report.value("moved").toInt();
  if (moved == 0) {
    s += "**" + tr("The load order was already sorted.") + "**\n";
  } else {
    s += "**" + tr("The load order changed: %1 plugin(s) moved.").arg(moved) + "**\n";
  }
  const QString note = report.value("masterlistNote").toString();
  if (!note.isEmpty()) {
    s += "\n**" + tr("Warning") + "**: " +
         tr("the masterlist was not updated (%1); the copy from before was used.").arg(note) + "\n";
  }
  return s;
}

// Loot::Report::successMarkdown
QString successMarkdown(const QJsonObject& report)
{
  QString s;
  const QJsonArray messages = report.value("messages").toArray();
  if (!messages.isEmpty()) {
    s += "### " + tr("General messages") + "\n";
    for (const auto& m : messages) {
      s += " - " + messageMarkdown(m.toObject()) + "\n";
    }
  }
  const QJsonArray plugins = report.value("plugins").toArray();
  if (!plugins.isEmpty()) {
    if (!s.isEmpty()) {
      s += "\n";
    }
    s += "### " + tr("Plugins") + "\n";
    for (const auto& p : plugins) {
      const QString ps = pluginMarkdown(p.toObject());
      if (!ps.isEmpty()) {
        s += ps + "\n";
      }
    }
  }
  if (s.isEmpty()) {
    s += "**" + tr("No messages.") + "**\n";
  }
  s += "\n" + statsMarkdown(report.value("stats").toObject());
  return outcomeMarkdown(report) + "\n" + s;
}

// Loot::Report::errorsMarkdown
QString errorsMarkdown(const QStringList& errors, const QStringList& warnings)
{
  QString s;
  if (!errors.isEmpty()) {
    s += "### " + tr("Errors") + ":\n";
    for (const QString& e : errors) {
      s += " - " + e + "\n";
    }
  }
  if (!warnings.isEmpty()) {
    if (!s.isEmpty()) {
      s += "\n";
    }
    s += "### " + tr("Warnings") + ":\n";
    for (const QString& w : warnings) {
      s += " - " + w + "\n";
    }
  }
  return s;
}

}  // namespace

QString reportMarkdown(bool okay, const QStringList& errors, const QStringList& warnings,
                       const QJsonObject& report)
{
  QString s;
  if (!okay) {
    s += "## " + tr("Loot failed to run") + "\n";
    if (errors.isEmpty() && warnings.isEmpty()) {
      s += tr("No errors were reported. The log below might have more information.\n");
    }
  }
  s += errorsMarkdown(errors, warnings);
  if (okay) {
    s += "\n" + successMarkdown(report);
  }
  return s;
}

QString pluginMarkdown(const QJsonObject& plugin)
{
  QString s;
  const QJsonArray incompatibilities = plugin.value("incompatibilities").toArray();
  if (!incompatibilities.isEmpty()) {
    s += " - **" + tr("Incompatibilities") + ": " + joined(incompatibilities) + "**\n";
  }
  const QJsonArray missingMasters = plugin.value("missingMasters").toArray();
  if (!missingMasters.isEmpty()) {
    s += " - **" + tr("Missing masters") + ": " + joined(missingMasters) + "**\n";
  }
  // Strata's addition: lootcli leaves requirements out
  const QJsonArray missingRequirements = plugin.value("missingRequirements").toArray();
  if (!missingRequirements.isEmpty()) {
    s += " - **" + tr("Missing requirements") + ": " + joined(missingRequirements) + "**\n";
  }
  for (const auto& m : plugin.value("messages").toArray()) {
    s += " - " + messageMarkdown(m.toObject()) + "\n";
  }
  for (const auto& d : plugin.value("dirty").toArray()) {
    s += " - " + dirtyString(d.toObject(), false) + "\n";
  }
  if (!s.isEmpty()) {
    s = "#### " + plugin.value("name").toString() + "\n" + s;
  }
  return s;
}

QString dirtyString(const QJsonObject& dirty, bool isClean)
{
  QString utility = dirty.value("cleaningUtility").toString();
  if (utility.isEmpty()) {
    utility = "?";
  }
  if (isClean) {
    return tr("Verified clean by %1").arg(utility);
  }
  // Loot::Dirty::cleaningString
  QString s = tr("%1 found %2 ITM record(s), %3 deleted reference(s) and %4 deleted navmesh(es).")
                  .arg(utility)
                  .arg(dirty.value("itm").toInteger())
                  .arg(dirty.value("deletedReferences").toInteger())
                  .arg(dirty.value("deletedNavmesh").toInteger());
  const QString info = dirty.value("info").toString();
  if (!info.isEmpty()) {
    s += " " + info;
  }
  return s;
}

QString pluginTooltip(const QJsonObject& plugin)
{
  QString s;
  for (const auto& f : plugin.value("incompatibilities").toArray()) {
    s += "<li>" + tr("Incompatible with %1").arg(fileName(f).toHtmlEscaped()) + "</li>";
  }
  for (const auto& m : plugin.value("missingMasters").toArray()) {
    s += "<li>" + tr("Depends on missing %1").arg(m.toString().toHtmlEscaped()) + "</li>";
  }
  for (const auto& f : plugin.value("missingRequirements").toArray()) {
    s += "<li>" + tr("Requires missing %1").arg(fileName(f).toHtmlEscaped()) + "</li>";
  }
  for (const auto& v : plugin.value("messages").toArray()) {
    const QJsonObject m = v.toObject();
    s += "<li>";
    const QString type = m.value("type").toString();
    if (type == "warn") {
      s += tr("Warning") + ": ";
    } else if (type == "error") {
      s += tr("Error") + ": ";
    }
    s += m.value("text").toString() + "</li>";
  }
  for (const auto& d : plugin.value("dirty").toArray()) {
    s += "<li>" + dirtyString(d.toObject(), false) + "</li>";
  }
  for (const auto& c : plugin.value("clean").toArray()) {
    s += "<li>" + dirtyString(c.toObject(), true) + "</li>";
  }
  if (!s.isEmpty()) {
    s = "<hr><ul style=\"margin-top:0px; padding-top:0px; margin-left:15px; "
        "-qt-list-indent: 0;\">" +
        s + "</ul>";
  }
  return s;
}

QStringList pluginIcons(QStringList icons, const QJsonObject& plugin)
{
  const QString warning = ":/MO/gui/warning";
  const QString locked  = ":/MO/gui/locked";
  const QString info    = ":/MO/gui/information";
  // PluginList::isProblematic
  const bool problematic = !plugin.value("incompatibilities").toArray().isEmpty() ||
                           !plugin.value("missingMasters").toArray().isEmpty() ||
                           !plugin.value("missingRequirements").toArray().isEmpty();
  if (problematic && !icons.contains(warning)) {
    icons.prepend(warning);
  }
  // PluginList::hasInfo: after the warning and the lock
  if (!plugin.value("messages").toArray().isEmpty() && !icons.contains(info)) {
    qsizetype at = 0;
    if (icons.value(at) == warning) {
      ++at;
    }
    if (icons.value(at) == locked) {
      ++at;
    }
    icons.insert(at, info);
  }
  if (!plugin.value("dirty").toArray().isEmpty()) {
    icons.append(":/MO/gui/edit_clear");
  }
  return icons;
}

}  // namespace loot
