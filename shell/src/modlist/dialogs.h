#pragma once

// Entry points of the modlist area's dialogs (round 2). The main window calls these; each
// works on the instance folder `root` through the core (strata_bridge/bridge_modlist.h)
// and returns whether something changed that the window must read again.

#include <QString>

class QWidget;

namespace strata::modlist
{

// The profiles dialog (MO2's ProfilesDialog). `selected` receives the profile picked with
// "Select" (already made the instance's selected profile, a live deploy remounted), or
// stays empty.
bool showProfilesDialog(QWidget* parent, const QString& root, QString* selected);

// The executables dialog (MO2's EditExecutablesDialog) for `profile` ("" = the selected
// one; the "create files in mod" choice is stored per profile).
bool showExecutablesDialog(QWidget* parent, const QString& root, const QString& profile);

// The settings dialog (MO2's SettingsDialog, the tabs that apply on Linux, plus Leet's).
bool showSettingsDialog(QWidget* parent, const QString& root);

// The ini editor (MO2's tool_inieditor) for `profile` ("" = the selected one). Returns
// whether a file was written.
bool showIniEditor(QWidget* parent, const QString& root, const QString& profile);

// The Problems dialog ("Notifications"). Returns whether a fix changed the plugin list.
bool showProblemsDialog(QWidget* parent, const QString& root, const QString& profile);

// The Crash Log Viewer (crashlogtools; labels the logs first when the Crash Log Labeler is on).
void showCrashLogViewer(QWidget* parent, const QString& root, const QString& profile);

// Opens one of the above by name ("profiles", "executables", "settings", "delete",
// "inieditor", "problems", "crashlogs") for the
// test hook STRATA_SHELL_DIALOG. With STRATA_SHELL_DIALOG_SCREENSHOT=<png> the dialog is
// shown without blocking, saved as a picture after STRATA_SHELL_SCREENSHOT_DELAY ms
// (default 800) and the program quits.
void openDialogByName(QWidget* parent, const QString& name, const QString& root,
                      const QString& profile);

}  // namespace strata::modlist
