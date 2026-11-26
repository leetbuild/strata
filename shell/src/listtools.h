#pragma once

// Three of MO2's mod list actions on the Rust core (strata_ui::listtools): the "Export to
// csv" dialog with its SaveTextAsDialog, and Overwrite's "Sync to Mods..." dialog.

#include <QStringList>

class QWidget;

namespace strata
{
struct Session;
}

namespace listtools
{

// ModListViewActions::exportModListCSV: the options, then the text to copy or save.
// `visible` = the mods the list shows now (the third choice).
void exportCsv(QWidget* parent, strata::Session& session, const QStringList& visible);

// OrganizerCore::syncOverwrite: SyncOverwriteDialog, then the moves. Returns whether
// anything was moved.
bool syncOverwrite(QWidget* parent, strata::Session& session);

}  // namespace listtools
